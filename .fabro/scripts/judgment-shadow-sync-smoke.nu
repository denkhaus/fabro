#!/usr/bin/env nu
# judgment-shadow sync battery (fabro-d367 heredoc era; INVERTED by
# fabro-091b): pins the closure transport contract of the judgment-shadow
# host hook. The ONE source of truth is .fabro/scripts/judgment-shadow.nu;
# each of the four loop workflows DECLARES that file on its
# [[run.hooks]] entry (`files = [...]`), the bundler carries it in the
# workflow closure, and the server stages it under FABRO_HOOK_ASSETS for
# the worker — a host-side hook has no workspace cwd on the Docker
# provider, so the inline-heredoc workaround is gone.
#
# Battery (pure logic + two sh -c simulations; no network, no cargo):
#   1. every lane's judgment-shadow hook is exactly one, stays host-side
#      (sandbox = false), declares the script file, and carries the path
#      form of the transport (no embedded payload anywhere);
#   2. every sandbox = false hook in the four lanes declares files — the
#      closure is the only transport a host-side hook has;
#   3. the path form RUNS under the engine's host-hook shape: staged
#      assets via FABRO_HOOK_ASSETS with cwd outside the repo (worker
#      case), and unset with the repo as cwd (local CLI case) — both write
#      one fabro-judgment-v1 line.
#
# Exit 0 = green; any red prints and exits 1.

const SOURCE = '.fabro/scripts/judgment-shadow.nu'
const LANES = [loop develop conductor revisor]
const SCRIPT_FORM = 'nu "${FABRO_HOOK_ASSETS:-.}/.fabro/scripts/judgment-shadow.nu"'
const DECLARED_FILE = '../../scripts/judgment-shadow.nu'

def fail [msg: string]: nothing -> nothing {
    print -e $"judgment-shadow-sync: RED — ($msg)"
    exit 1
}

def main []: nothing -> nothing {
    if not ($SOURCE | path exists) { fail $"source file ($SOURCE) is missing" }

    # 1. the judgment-shadow hook in every lane
    for lane in $LANES {
        let manifest = $"./.fabro/workflows/($lane)/workflow.toml"
        let raw = (open --raw $manifest)
        if ($raw | str contains 'JSEOF') { fail $"($manifest): heredoc payload still present — the fabro-091b path form replaced it" }
        let hooks = (open $manifest | get run.hooks | where name == 'judgment-shadow')
        if ($hooks | length) != 1 { fail $"($manifest): expected exactly one judgment-shadow hook" }
        let hook = $hooks.0
        if $hook.sandbox != false { fail $"($manifest): judgment-shadow must stay host-side \(sandbox = false — the OpenRouter key lives in the server env)" }
        if ($hook.files? | default [] | where {|f| $f == $DECLARED_FILE} | is-empty) {
            fail $"($manifest): judgment-shadow must declare ($DECLARED_FILE) in files — the closure is its only transport"
        }
        if $hook.script != $SCRIPT_FORM { fail $"($manifest): script must be the fabro-091b path form, got ($hook.script)" }

        # 2. every host-side hook declares files
        let host_hooks = (open $manifest | get run.hooks | where {|h| $h.sandbox? == false})
        for h in $host_hooks {
            if ($h.files? | default [] | is-empty) {
                fail $"($manifest): host-side hook \(($h.name)) declares no files — a sandbox = false hook cannot reach workspace files any other way"
            }
        }
    }

    # 3a. worker shape: staged assets, cwd outside the repo
    let tmp = (mktemp -d -t 'jssync.XXXXXX')
    let worker_ok = (simulate $tmp 'sync-battery' $"($tmp)/assets" null)
    rm -rf $tmp
    if not $worker_ok.0 { fail $"worker-shape simulation failed — ($worker_ok.1)" }

    # 3b. local CLI shape: FABRO_HOOK_ASSETS unset, repo root as cwd
    let tmp2 = (mktemp -d -t 'jssync.XXXXXX')
    let local_ok = (simulate $tmp2 'sync-battery-local' null true)
    rm -rf $tmp2
    if not $local_ok.0 { fail $"local-shape simulation failed — ($local_ok.1)" }

    print 'judgment-shadow-sync: GREEN (4 lanes on the closure transport, worker + local shapes simulated green)'
}

# Runs the workflow's exact script string under `sh -c` with a context
# file and an empty OpenRouter key. The worker shape stages assets under
# `$assets` and runs with cwd `$tmp` (no workspace); the local shape
# leaves the variable unset and runs inside `$tmp/ws`, a throwaway
# workspace carrying the same staged layout a checkout would. Returns
# [ok, detail].
def simulate [tmp: string, run_id: string, assets?, workspace?] {
    let cwd = (if ($workspace | default false) { $"($tmp)/ws" } else { $tmp })
    let stream = (if ($workspace | default false) { $"($cwd)/.fabro/judgments/($run_id).jsonl" } else { $"($tmp)/storage/judgments/($run_id).jsonl" })
    mkdir $"($tmp)/assets/.fabro/scripts"
    open --raw $SOURCE | save --raw $"($tmp)/assets/.fabro/scripts/judgment-shadow.nu"
    if ($workspace | default false) {
        mkdir $"($cwd)/.fabro/scripts"
        open --raw $SOURCE | save --raw $"($cwd)/.fabro/scripts/judgment-shadow.nu"
    }
    {event: stage_complete, run_id: $run_id, node_id: reviewer, context_updates: {anomaly_files: [a.rs]}} | to json | save --raw $"($tmp)/ctx.json"
    let script = (open ./.fabro/workflows/loop/workflow.toml | get run.hooks | where name == 'judgment-shadow' | get 0.script)
    let env_base = {FABRO_HOOK_CONTEXT: $"($tmp)/ctx.json", FABRO_STORAGE_DIR: $"($tmp)/storage", OPENROUTER_API_KEY: ''}
    let hook_env = (if $assets == null { $env_base } else { $env_base | insert FABRO_HOOK_ASSETS $assets })
    let res = (
        with-env $hook_env {
            if $assets == null { hide-env FABRO_HOOK_ASSETS --ignore-errors }
            cd $cwd
            do { ^sh -c $"($script)\n__fabro_status=\$?\nexit \$__fabro_status" } | complete
        }
    )
    let line_ok = (
        if not ($stream | path exists) { false } else {
            let entry = (open --raw $stream | lines | last | from json)
            ($entry.schema == 'fabro-judgment-v1') and ($entry.node == 'reviewer') and ($entry.degraded == 'no_key')
        }
    )
    if not $line_ok {
        [false, $"no valid line at ($stream) — exit \(($res.exit_code)), stderr: ($res.stderr | str substring 0..200)"]
    } else if $res.exit_code != 0 {
        [false, $"exit \(($res.exit_code)) — stderr: ($res.stderr | str substring 0..200)"]
    } else {
        [true, '']
    }
}
