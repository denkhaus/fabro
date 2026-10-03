#!/usr/bin/env nu
# Loop graph-contract smoke (fabro-70b5): pins the loop lane's
# load-bearing graph contract — pure file assertions, no seeds, no git
# (develop graph-contract-smoke pattern, wired into scripts/qualitygate.nu's
# loop-asset tier).
#
#   nu .fabro/workflows/loop/scripts/graph-contract-smoke.nu
#
# Contract:
#   1. planner node carries NO goal_gate — the deterministic guards
#      (tracker_guard) legitimately bypass it (same fabro-92e2 class as
#      develop).
#   2. NO preflight -> exit edge — report-only preflight, never a script
#      exit.
#   3. tracker_guard -> exit ("Tracker empty") edge EXISTS.
#   4. preflight -> planner ("Preflight done") edge EXISTS.
#   5. planner -> exit ("Already landed") edge EXISTS.
#   6. Lane wiring: tracker_guard and preflight script lines carry
#      --assignee loop; evidence and closeout carry --lane loop — a
#      dropped flag would silently work @fabro's queue / capture the
#      product review scope.
#   7. Implementer envelope: x.fs_write names EXACTLY the loop-asset
#      set (.fabro/**, scripts/**, justfile, .seeds/issues.jsonl) plus
#      the fabro-dot snapshot dir (fabro-9973 — the one lib/ exception,
#      mechanically derived from .fabro/workflows graphs) and fs_hide
#      hides .agents/** (the meta lane must SEE loop assets).
#   8. tester -> implementer ("Gate red") edge EXISTS — the loop lane has
#      no gatebounce node; the red bounce returns straight to the
#      implementer.
#   9. The tester runs the loop gate, NOT the product gate
#      (just qualitygate must not appear on the tester node).
#  10. Reviewer evidence line budget: x.preamble_output_max_lines on the
#      reviewer node stays >= 1000 (fabro-31f7) — the stage Output tail
#      cap must never silently omit an evidence capture's head.

const GRAPH = ('.fabro/workflows/loop/workflow.fabro' | path expand)
const FS_WRITE_EXPECTED = 'x.fs_write=".fabro/**,scripts/**,justfile,.seeds/issues.jsonl,.seeds/config.yaml,lib/components/fabro-dot/src/snapshots/**"'

def fail [what: string]: nothing -> nothing {
    print -e $"loop graph-contract-smoke: FAIL — ($what)"
    exit 1
}

def main [] {
    if not ($GRAPH | path exists) { fail $"graph not found: ($GRAPH)" }
    let text = (open --raw $GRAPH)
    let lines = ($text | lines)

    # 1. planner block must not carry goal_gate (comments stripped)
    let start = ($lines | enumerate | where {|e| ($e.item | str trim) == 'planner ['} | get -o index | first | default null)
    if $start == null { fail 'planner node not found' }
    let end = ($lines | enumerate | where {|e| ($e.index > $start) and (($e.item | str trim) == ']')} | get -o index | first | default null)
    if $end == null { fail 'planner node block not terminated' }
    let block = ($lines | skip ($start + 1) | take ($end - $start - 1))
    if ($block | any {|l| ($l | split row '//' | first | str contains 'goal_gate')}) {
        fail 'planner node carries goal_gate: deterministic guard exits would die at the terminal gate check'
    }

    if ($lines | any {|l| ($l | str contains 'preflight -> exit')}) {
        fail 'preflight -> exit edge exists: report-only preflight must never exit the run'
    }
    if not ($lines | any {|l| ($l | str contains 'tracker_guard -> exit') and ($l | str contains 'Tracker empty')}) {
        fail 'tracker_guard -> exit ("Tracker empty") edge missing'
    }
    if not ($lines | any {|l| ($l | str contains 'preflight -> planner') and ($l | str contains 'Preflight done')}) {
        fail 'preflight -> planner ("Preflight done") edge missing'
    }
    if not ($lines | any {|l| ($l | str contains 'planner -> exit') and ($l | str contains 'Already landed')}) {
        fail 'planner -> exit ("Already landed") edge missing'
    }

    # 6. lane wiring on the script lines
    if not ($lines | any {|l| ($l | str contains 'tracker-guard.nu') and ($l | str contains '--assignee loop')}) {
        fail 'tracker_guard script line lost --assignee loop (would work @fabro queue)'
    }
    if not ($lines | any {|l| ($l | str contains 'planner-preflight.nu') and ($l | str contains '--assignee loop')}) {
        fail 'preflight script line lost --assignee loop'
    }
    if not ($lines | any {|l| ($l | str contains 'evidence.nu') and ($l | str contains '--lane loop')}) {
        fail 'evidence script line lost --lane loop (product review scope would ship)'
    }
    if not ($lines | any {|l| ($l | str contains 'closeout.nu') and ($l | str contains '--lane loop')}) {
        fail 'closeout script line lost --lane loop (residuals would file @fabro)'
    }

    # 7. implementer envelope pins
    let istart = ($lines | enumerate | where {|e| ($e.item | str trim) == 'implementer ['} | get -o index | first | default null)
    if $istart == null { fail 'implementer node not found' }
    let iend = ($lines | enumerate | where {|e| ($e.index > $istart) and (($e.item | str trim) == ']')} | get -o index | first | default null)
    if $iend == null { fail 'implementer node block not terminated' }
    let iblock = ($lines | skip ($istart + 1) | take ($iend - $istart - 1) | each {|l| $l | split row '//' | first | str trim | str trim -r -c ','})
    if not ($iblock | any {|l| $l == $FS_WRITE_EXPECTED}) {
        fail $"implementer x.fs_write drifted — expected exactly ($FS_WRITE_EXPECTED) \(the a9bb protection surface\)"
    }
    if not ($iblock | any {|l| ($l | str starts-with 'x.fs_hide=') and ($l | str contains '.agents/')}) {
        fail 'implementer x.fs_hide lost .agents/ — the meta lane must still NOT see/edit session-level skills'
    }

    # 8. gate-red bounce edge
    if not ($lines | any {|l| ($l | str contains 'tester -> implementer') and ($l | str contains 'Gate red')}) {
        fail 'tester -> implementer ("Gate red") edge missing — red bounces would dead-end'
    }

    # 9. tester runs the loop gate, never the product gate
    if ($lines | any {|l| ($l | str contains 'just qualitygate')}) {
        fail 'graph names the product gate (just qualitygate) — the loop tester owns loop-gate.nu only'
    }
    if not ($lines | any {|l| ($l | str contains 'loop-gate.nu')}) {
        fail 'tester script line lost loop-gate.nu'
    }

    # 10. reviewer evidence line budget (fabro-31f7): the stage Output
    # render is a TAIL cap, and evidence.nu puts the integrity header +
    # seed-work diff at the capture HEAD — a low cap silently drops
    # exactly that head ("(275 lines omitted)" hid new-script heads from
    # a reviewer). The reviewer's x.preamble_output_max_lines must stay
    # >= 1000 so every realistic capture renders whole; pathological
    # sizes are evidence.nu's own HARD_CAP disclosure, never this cap.
    let rstart = ($lines | enumerate | where {|e| ($e.item | str trim) == 'reviewer ['} | get -o index | first | default null)
    if $rstart == null { fail 'reviewer node not found' }
    let rend = ($lines | enumerate | where {|e| ($e.index > $rstart) and (($e.item | str trim) == ']')} | get -o index | first | default null)
    if $rend == null { fail 'reviewer node block not terminated' }
    let rblock = ($lines | skip ($rstart + 1) | take ($rend - $rstart - 1) | each {|l| $l | split row '//' | first | str trim | str trim -r -c ','})
    let cap = ($rblock | where {|l| $l | str starts-with 'x.preamble_output_max_lines='} | first | default null)
    if $cap == null {
        fail 'reviewer node lost x.preamble_output_max_lines — the engine default tail cap would silently omit evidence heads'
    }
    let value = ($cap | str replace --regex '\D+' '' | into int)
    if $value < 1000 {
        fail $"reviewer x.preamble_output_max_lines=($value) is below the fabro-31f7 floor of 1000 — evidence heads would be silently omitted"
    }

    print 'loop graph-contract-smoke: OK — guard exits, lane wiring, envelope pins, red bounce, evidence budget intact'
}
