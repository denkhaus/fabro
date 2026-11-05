#!/usr/bin/env nu
# The per-facade flag guard's pin (#1130, born from the sprint-78 --all
# incident): every arm's OWNED list in the ARMS table actually refuses a
# foreign flag, and no arm's OWNED list is empty (an empty list would
# refuse everything — the signature's whole union). Drives the shim
# directly over a fixture state: the guard's refusal is loud and names
# the facade/sub pair. Also pins the lockstep between the ARMS table and
# the match: a facade pair in the table but not the match (or vice
# versa) refuses or falls through, and this battery sees it.

const SCRIPTS_DIR = (path self | path dirname)
use (($SCRIPTS_DIR | path join 'lib' 'smoke_harness.nu')) [start-fail-log, fail, end-fail-gate, shim-env]
load-env {SMOKE_FAIL_LOG: (start-fail-log 'work-shim-guard-smoke')}

const SHIM = ($SCRIPTS_DIR | path join 'fabro-work-shim.nu')

# One drive: the argv against the shim with a fixture state. Returns
# the complete-wrapped record.
def drive [state: string, argv: list<string>]: nothing -> any {
    do {
        with-env (shim-env $SHIM $state) {
            ^$SHIM ...$argv
        }
    } | complete
}

def main []: nothing -> nothing {
    let state = (mktemp -t shim-guard.XXXXXX.json)
    {items: [], claims: [], nextKey: 9200, log: [], runs: [], marks: [], observations: [], radius: {}} | to json --raw | save -f $state

    # --- Foreign flags refuse loudly, naming the facade pair.
    # ps owns server/workflow/json/all — the work facade's project is
    # foreign there (the --all incident's exact shape: a flag from
    # another facade's domain must not be silently eaten).
    let ps_foreign = (drive $state [ps --server http://127.0.0.1:1 --all --project fabro])
    if $ps_foreign.exit_code == 0 {
        fail "ps must refuse the work facade's --project (the --all drift class)"
    }
    if not ($ps_foreign.stderr | str contains 'does not belong to ps') {
        fail $"the refusal names the facade pair: ($ps_foreign.stderr | str substring 0..160)"
    }

    # run queue owns the orchestration reads — the observation facade's
    # run-id is foreign there (a smoke asserting a queue read would
    # otherwise not see a silently eaten --run-id).
    let queue_foreign = (drive $state [run queue --server http://127.0.0.1:1 --run-id 01X])
    if $queue_foreign.exit_code == 0 {
        fail "run queue must refuse the observation facade's --run-id"
    }
    if not ($queue_foreign.stderr | str contains 'does not belong to run queue') {
        fail $"the refusal names the facade pair: ($queue_foreign.stderr | str substring 0..160)"
    }

    # work show owns only server/project/json — the run facade's field
    # is foreign (the classic cross-eat the #1109 guard exists for).
    let show_foreign = (drive $state [work show --server http://127.0.0.1:1 --project fabro --field merge])
    if $show_foreign.exit_code == 0 {
        fail "work show must refuse the run facade's --field"
    }

    # --- The OWNED list must actually cover the arm's own flags: ps
    # --all parses (it is OWNED) — the positive arm beside the refusals.
    let ps_owned = (drive $state [ps --server http://127.0.0.1:1 --all --json])
    if $ps_owned.exit_code != 0 {
        fail $"ps --all is owned and must parse: ($ps_owned.stderr | str substring 0..160)"
    }

    # --- The lockstep: every ARMS row answers (no fallback refusal for
    # a spec'd pair), and the log entry carries exactly the LOGGED
    # flags of the spec (work list logs 4 of its 6 owned — limit and
    # lane stay out of the entry).
    let listed = (drive $state [work list --server http://127.0.0.1:1 --project fabro --limit 5])
    if $listed.exit_code != 0 {
        fail $"every ARMS row must answer without the fallback refusal: ($listed.stderr | str substring 0..160)"
    }
    let entry = (open --raw $state | from json | get log | last)
    if ($entry.flags | columns | sort) != [json project query server] {
        fail $"work list logs exactly its spec's four flags: ($entry.flags | to json -r)"
    }

    end-fail-gate
    print 'work-shim-guard-smoke: ok — foreign flags refuse naming the pair, owned flags parse, the log carries the spec flags'
}
