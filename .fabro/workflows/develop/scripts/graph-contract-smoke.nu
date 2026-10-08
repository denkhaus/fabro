#!/usr/bin/env nu
# Graph-contract smoke (fabro-83df / fabro-92e2, 2026-09-19): pins the
# develop graph's deterministic-exit contract that the 2026-09-19
# goal-gate incident broke. Pure file assertions, no seeds, no git — the
# same closeout-smoke pattern (checked into the repo, wired into
# scripts/qualitygate.nu's loop-asset tier).
#
#   nu .fabro/workflows/develop/scripts/graph-contract-smoke.nu
#
# Contract:
#   1. planner node carries NO goal_gate — the deterministic guards
#      (tracker_guard) legitimately bypass it; a gate made those runs
#      die with "goal gate unsatisfied for node planner".
#   2. NO preflight -> exit edge — the preflight is report-only
#      (fabro-83df); a landed-but-open top candidate is the planner's
#      decision, never a script exit.
#   3. tracker_guard -> exit ("Tracker empty") edge EXISTS — the
#      zero-token drained-tracker exit must stay.
#   4. preflight -> planner ("Preflight done") edge EXISTS — the
#      unconditional fail-open route must stay.
#   5. planner -> exit ("Already landed") edge EXISTS — the planner's
#      own decision exit must stay.
#   6. Toolchain-placement guard (fabro-2357, the loop-lane pattern of
#      fabro-3ab2): start routes through env_guard before tracker_guard,
#      the node runs the shared guard through POSIX sh with the develop
#      lane flag, and start has NO second outgoing edge. A dropped or
#      bypassed guard silently returns the cryptic "nu: command not
#      found" death on a flag-less manual fire.

const GRAPH = ('.fabro/workflows/develop/workflow.fabro' | path expand)

def fail [what: string]: nothing -> nothing {
    print -e $"graph-contract-smoke: FAIL — ($what)"
    exit 1
}

def main [] {
    if not ($GRAPH | path exists) { fail $"graph not found: ($GRAPH)" }
    let text = (open --raw $GRAPH)
    let lines = ($text | lines)

    # planner node block: from 'planner [' to the next lone ']'
    let start = ($lines | enumerate | where {|e| ($e.item | str trim) == 'planner ['} | get -o index | first | default null)
    if $start == null { fail 'planner node not found' }
    let end = ($lines | enumerate | where {|e| $e.index > $start and ($e.item | str trim) == ']'} | get -o index | first | default null)
    if $end == null { fail 'planner node block not terminated' }
    let block = ($lines | skip ($start + 1) | take ($end - $start - 1))
    # strip inline comments before matching — the rationale comment
    # legitimately names goal_gate, only the attribute is a regression
    if ($block | any {|l| ($l | split row '//' | first | str contains 'goal_gate')}) {
        fail 'planner node carries goal_gate (fabro-92e2 regression): deterministic guard exits would die at the terminal gate check'
    }

    if ($lines | any {|l| ($l | str contains 'preflight -> exit')}) {
        fail 'preflight -> exit edge exists (fabro-83df regression): report-only preflight must never exit the run'
    }
    if not ($lines | any {|l| ($l | str contains 'tracker_guard -> exit') and ($l | str contains 'Tracker empty')}) {
        fail 'tracker_guard -> exit ("Tracker empty") edge missing: zero-token drained-tracker exit lost'
    }
    if not ($lines | any {|l| ($l | str contains 'preflight -> planner') and ($l | str contains 'Preflight done')}) {
        fail 'preflight -> planner ("Preflight done") edge missing: unconditional fail-open route lost'
    }
    if not ($lines | any {|l| ($l | str contains 'planner -> exit') and ($l | str contains 'Already landed')}) {
        fail 'planner -> exit ("Already landed") edge missing: the planner decision exit lost'
    }
    # 6. toolchain-placement guard (fabro-2357): start -> env_guard ->
    # tracker_guard, run through POSIX sh with the develop lane flag.
    # Assertions read COMMENT-STRIPPED lines (the loop smoke's precedent):
    # the rationale comments name the contract, and only real code may
    # satisfy it.
    let code = ($lines | each {|l| ($l | split row '//' | first)})
    if not ($code | any {|l| ($l | str contains 'start -> env_guard')}) {
        fail 'start -> env_guard edge missing — a misplaced manual fire would die cryptically at the first nu stage'
    }
    if not ($code | any {|l| ($l | str contains 'env_guard -> tracker_guard')}) {
        fail 'env_guard -> tracker_guard edge missing — the placement guard must precede every nu stage'
    }
    if not ($code | any {|l| ($l | str contains 'toolchain-guard.sh develop') and ($l | str starts-with '        script="sh ')}) {
        fail 'env_guard script line must run the shared guard via POSIX sh with the develop lane flag — nu cannot guard a sandbox without nu'
    }
    let start_edges = ($code | where {|l| ($l | str trim | str starts-with 'start ->')})
    if ($start_edges | length) != 1 {
        fail $'start must have exactly ONE outgoing edge, the placement guard; found ($start_edges | length) — a second edge would bypass it'
    }
    print 'graph-contract-smoke: OK — planner ungated, preflight report-only, guard exits intact, placement guard wired'
}
