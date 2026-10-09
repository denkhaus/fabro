#!/usr/bin/env nu
# judgment-shadow sync battery (fabro-d367): pins the heredoc transport
# contract of the judgment-shadow host hook. The ONE source of truth is
# .fabro/scripts/judgment-shadow.nu; each of the four loop workflows
# embeds that file's exact bytes inside its [[run.hooks]] script string
# (a quoted heredoc fed through `nu -c`), because a host-side hook runs
# with no workspace cwd whenever the run's sandbox does not share the
# host filesystem (the Docker provider — the packaged server container
# has none of the repo's files, which is why the old repo-relative
# `nu .fabro/scripts/judgment-shadow.nu` exited 1 on every stage,
# silently, for the whole ADR-0022 wave).
#
# Battery (pure logic + one sh -c simulation; no network, no cargo):
#   1. the source file is embeddable: no run of three single quotes
#      (TOML literal-string terminator) and no bare JSEOF line;
#   2. each workflow's embedded body is byte-identical to the source;
#   3. the extracted command actually RUNS under the engine's host-hook
#      shape (`sh -c`, no workspace cwd, context via FABRO_HOOK_CONTEXT,
#      storage-anchored stream): exit 0 and one fabro-judgment-v1 line.
#
# Exit 0 = green; any red prints and exits 1.

const SOURCE = '.fabro/scripts/judgment-shadow.nu'
const LANES = [loop develop conductor revisor]
const MARKER_START = "nu -c \"$(cat <<'JSEOF'"

def fail [msg: string]: nothing -> nothing {
    print -e $"judgment-shadow-sync: RED — ($msg)"
    exit 1
}

def embedded_body [script: string] {
    # The command is: nu -c "$(cat <<'JSEOF'\n<body>\nJSEOF\n)"
    let start = ($script | str index-of $MARKER_START)
    if $start < 0 { return null }
    let body_start = ($start + ($MARKER_START | str length) + 1)
    # find the delimiter line AFTER the body
    let tail = ($script | str substring $body_start..)
    let end_rel = ($tail | lines | enumerate | where {|it| $it.item == 'JSEOF'} | get -o index.0)
    if $end_rel == null { return null }
    let kept = ($tail | lines | first $end_rel)
    $kept | str join "\n"
}

def main []: nothing -> nothing {
    let body = (open --raw $SOURCE | str trim -r -c "\n")

    # 1. embeddability of the source
    if ($body | str contains "'''") { fail "source contains a three-single-quote run (TOML literal-string terminator)" }
    if ($body | lines | where {|l| $l == 'JSEOF'} | is-not-empty) { fail "source contains a bare JSEOF line (heredoc delimiter)" }

    # 2. byte-equality in every lane
    for lane in $LANES {
        let manifest = $"./.fabro/workflows/($lane)/workflow.toml"
        let hooks = (open $manifest | get run.hooks | where name == 'judgment-shadow')
        if ($hooks | length) != 1 { fail $"($manifest): expected exactly one judgment-shadow hook" }
        let hook = $hooks.0
        if $hook.sandbox != false { fail $"($manifest): judgment-shadow must stay host-side \(sandbox = false — the OpenRouter key lives in the server env)" }
        let embedded = (embedded_body $hook.script)
        if $embedded == null { fail $"($manifest): script string does not carry the JSEOF heredoc transport" }
        if $embedded != $body {
            fail $"($manifest): embedded copy drifted from ($SOURCE) — re-embed the file's bytes (edit the source, never the copy)"
        }
    }

    # 3. the transport runs under the engine's host-hook shape: sh -c,
    # cwd OUTSIDE the repo (no workspace cwd — the Docker-provider case),
    # context file via FABRO_HOOK_CONTEXT, stream anchored at
    # FABRO_STORAGE_DIR.
    let tmp = (mktemp -d -t 'jssync.XXXXXX')
    printf '%s' '{"event":"stage_complete","run_id":"sync-battery","node_id":"reviewer","context_updates":{"anomaly_files":["a.rs"]}}' | save --raw $"($tmp)/ctx.json"
    let script = (open ./.fabro/workflows/loop/workflow.toml | get run.hooks | where name == 'judgment-shadow' | get 0.script)
    let res = (
        with-env {FABRO_HOOK_CONTEXT: $"($tmp)/ctx.json", FABRO_STORAGE_DIR: $"($tmp)/storage", OPENROUTER_API_KEY: ''} {
            cd $tmp
            do { ^sh -c $"($script)\n__fabro_status=\$?\nexit \$__fabro_status" } | complete
        }
    )
    let stream = $"($tmp)/storage/judgments/sync-battery.jsonl"
    let line_ok = (
        if not ($stream | path exists) { false } else {
            let entry = (open --raw $stream | lines | last | from json)
            ($entry.schema == 'fabro-judgment-v1') and ($entry.node == 'reviewer') and ($entry.degraded == 'no_key')
        }
    )
    rm -rf $tmp
    if $res.exit_code != 0 { fail $"transport simulation exited (($res.exit_code)) — stderr: ($res.stderr | str substring 0..200)" }
    if not $line_ok { fail $"transport simulation wrote no valid degraded line — stdout: ($res.stdout | str substring 0..200)" }

    print 'judgment-shadow-sync: GREEN (source embeddable, 4 lanes byte-identical, transport simulated green)'
}
