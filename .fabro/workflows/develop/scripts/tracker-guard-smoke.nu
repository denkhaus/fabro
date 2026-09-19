#!/usr/bin/env nu
# Smoke test for tracker-guard.nu's PURE decision logic (fabro-0da8):
# guard-decision/sd-issue-count over canned `complete`-style records —
# both-empty route, non-empty routes (open and in_progress arms), and
# the sd-failure fail-open contract (non-zero exit, invalid JSON,
# success:false) — without shelling to sd (the live-path is a manual
# invocation check, closeout-smoke pattern). The `source` const
# resolves against THIS file's directory, so the script runs from any
# cwd:
#   nu .fabro/workflows/develop/scripts/tracker-guard-smoke.nu

const GUARD = "tracker-guard.nu"
source $GUARD

def fail [what: string]: nothing -> nothing {
    print -e $"tracker-guard-smoke: FAIL — ($what)"
    exit 1
}

def ok-empty []: nothing -> record {
    {"exit_code": 0, "stdout": '{"success":true,"command":"list","issues":[]}'}
}

def ok-issues [n: int]: nothing -> record {
    let items = (1..$n | each {|i| ('{"id":"fabro-x' + ($i | into string) + '"}') } | str join ",")
    {"exit_code": 0, "stdout": ('{"success":true,"command":"list","issues":[' + $items + ']}')}
}

def label-of [r: record]: nothing -> string {
    $r.preferred_next_label?
    | default ""
}

# Route: BOTH lists empty -> "Tracker empty" (the zero-token exit).
let both = (guard-decision (ok-empty) (ok-empty))
if (label-of $both) != "Tracker empty" { fail $"both-empty misrouted: ($both | to json -r)" }
if $both.outcome != "succeeded" { fail "both-empty outcome not succeeded" }

# Route: open non-empty (in_progress empty) -> planner path.
let open_route = (guard-decision (ok-issues 2) (ok-empty))
if (label-of $open_route) != "Tracker non-empty" { fail $"open-non-empty misrouted: ($open_route | to json -r)" }

# Route: in_progress non-empty (open empty) -> planner path — the
# active-run shape (a just-claimed seed) must never take the empty exit.
let inprog_route = (guard-decision (ok-empty) (ok-issues 1))
if (label-of $inprog_route) != "Tracker non-empty" { fail $"in-progress-non-empty misrouted: ($inprog_route | to json -r)" }

# Fail-open: non-zero sd exit (either call) -> planner path, succeeded.
let sd_dead = {"exit_code": 1, "stdout": "", "stderr": "boom"}
if (label-of (guard-decision $sd_dead (ok-empty))) != "Tracker non-empty" { fail "sd open-failure not fail-open" }
if (label-of (guard-decision (ok-empty) $sd_dead)) != "Tracker non-empty" { fail "sd in-progress-failure not fail-open" }

# Fail-open: invalid JSON stdout -> planner path.
let bad_json = {"exit_code": 0, "stdout": "not json at all"}
if (label-of (guard-decision $bad_json (ok-empty))) != "Tracker non-empty" { fail "invalid JSON not fail-open" }

# Fail-open: sd-reported success:false -> planner path.
let sd_err = {"exit_code": 0, "stdout": '{"success":false,"error":"tracker locked"}'}
if (label-of (guard-decision (ok-empty) $sd_err)) != "Tracker non-empty" { fail "success:false not fail-open" }

# Edge: absent `issues` key is a legitimate empty list, not a failure.
let no_issues_key = {"exit_code": 0, "stdout": '{"success":true,"command":"list"}'}
if (label-of (guard-decision $no_issues_key (ok-empty))) != "Tracker empty" { fail "absent issues key not treated as empty" }

# sd-issue-count unit checks: 0, n, -1 classes.
if (sd-issue-count (ok-empty)) != 0 { fail "sd-issue-count empty != 0" }
if (sd-issue-count (ok-issues 3)) != 3 { fail "sd-issue-count 3-issue != 3" }
if (sd-issue-count $sd_dead) != -1 { fail "sd-issue-count failure != -1" }
if (sd-issue-count $bad_json) != -1 { fail "sd-issue-count bad JSON != -1" }

print "tracker-guard-smoke: ok — both-empty, non-empty, and fail-open routes verified"

# Sourcing tracker-guard.nu imports its `def main`; nu auto-invokes it
# after the top level runs — exit explicitly so the smoke never shells
# to sd (closeout-smoke idiom).
exit 0
