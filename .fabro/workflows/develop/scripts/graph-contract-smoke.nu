#!/usr/bin/env nu
# Graph-contract smoke (fabro-83df / fabro-92e2, 2026-09-19; rewritten for
# the ADR-0032 phase-3 topology): pins the develop graph's deterministic
# contracts. Pure file assertions, no tracker, no git — the same
# closeout-smoke pattern (checked into the repo, wired into
# scripts/qualitygate.nu's loop-asset tier via .fabro/scripts/battery-runner.nu).
#
#   nu .fabro/workflows/develop/scripts/graph-contract-smoke.nu
#
# Contract:
#   1. planner node carries NO goal_gate — deterministic guard bypasses
#      made gated runs die; routing owns the flow.
#   2. NO preflight -> exit edge — the preflight is report-only
#      (fabro-83df); a landed-but-open item is adjudicated by roles,
#      never a script exit.
#   3. NO retired nodes: tracker_guard, claim_check, gatebounce must not
#      reappear (ADR-0032 phase 3 — host scripts cannot read the server-
#      side tracker; their arms folded into the planner tools and the
#      post-claim preflight).
#   4. start -> env_guard -> planner wiring: the placement guard is first
#      and start has NO second outgoing edge (fabro-2357).
#   5. The claim-contract chain: planner -> preflight ("Seed claimed"),
#      preflight reading the granted-item packet on stdin
#      (stdin_source="current_item_json", script line carries
#      --work-item), preflight -> implementer ("Preflight done"), and the
#      outcome=failed bounce back to the planner (fabro-c42f class).
#   6. planner -> exit ("Tracker empty") and ("Already landed") edges
#      EXIST — the drained-queue and superseded-close exits are the
#      planner's own since the tracker guard retired.
#   7. The work-item tool grants: the planner's x.fabro_tools names
#      fabro_work_next (the atomic claim), the closeout node's names
#      fabro_work_close, and the evidence node runs --work-item with the
#      packet on stdin (fork 7 context flow).

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
    # 7. the atomic claim grant is pinned on the planner node itself
    if not ($block | any {|l| ($l | str contains 'fabro_work_next')}) {
        fail 'planner x.fabro_tools missing fabro_work_next — the atomic claim grant (ADR-0032) must live on the planner node'
    }

    # 2. report-only preflight never exits the run
    if ($lines | any {|l| ($l | str contains 'preflight -> exit')}) {
        fail 'preflight -> exit edge exists (fabro-83df regression): report-only preflight must never exit the run'
    }

    # 3. retired nodes stay retired (comment-STRIPPED lines only — the
    # rationale comments name them by design)
    let code = ($lines | each {|l| ($l | split row '//' | first)})
    for node in ["tracker_guard" "claim_check" "gatebounce"] {
        if ($code | any {|l| ($l | str contains $node)}) {
            fail $"retired node ($node) reappeared in code — ADR-0032 phase 3 regression: its arms belong to the planner tools / post-claim preflight"
        }
    }

    # 4. toolchain-placement guard (fabro-2357): start -> env_guard ->
    # planner, POSIX sh guard script, single start edge.
    if not ($code | any {|l| ($l | str contains 'start -> env_guard')}) {
        fail 'start -> env_guard edge missing — a misplaced manual fire would die cryptically at the first nu stage'
    }
    if not ($code | any {|l| ($l | str contains 'env_guard -> planner')}) {
        fail 'env_guard -> planner edge missing — the placement guard must precede every LLM/nu stage'
    }
    if not ($code | any {|l| ($l | str contains 'toolchain-guard.sh develop') and ($l | str starts-with '        script="sh ')}) {
        fail 'env_guard script line must run the shared guard via POSIX sh with the develop lane flag — nu cannot guard a sandbox without nu'
    }
    let start_edges = ($code | where {|l| ($l | str trim | str starts-with 'start ->')})
    if ($start_edges | length) != 1 {
        fail $'start must have exactly ONE outgoing edge, the placement guard; found ($start_edges | length) — a second edge would bypass it'
    }

    # 5. claim-contract chain (fabro-c42f, ADR-0032 form)
    if not ($code | any {|l| ($l | str contains 'planner -> preflight') and ($l | str contains 'Seed claimed')}) {
        fail 'planner -> preflight ("Seed claimed") edge missing — the claimed route must pass the deterministic preflight'
    }
    if not ($code | any {|l| ($l | str contains 'preflight -> implementer') and ($l | str contains 'Preflight done')}) {
        fail 'preflight -> implementer ("Preflight done") edge missing — the unconditional fail-open route lost'
    }
    if not ($code | any {|l| ($l | str contains 'preflight -> planner') and ($l | str contains 'outcome=failed')}) {
        fail 'preflight -> planner bounce edge missing — a dropped granted-item packet must re-enter the planner, not park the run'
    }
    if not ($code | any {|l| ($l | str contains 'planner-preflight.nu --work-item')}) {
        fail 'preflight script line missing --work-item — the develop lane must run the granted-item mode, never the seeds candidate mode'
    }
    if not ($code | any {|l| ($l | str contains 'stdin_source="current_item_json"')}) {
        fail 'stdin_source="current_item_json" missing — the deterministic preflight/evidence nodes read the granted-item packet, not a tracker id'
    }

    # 6. planner decision exits
    if not ($lines | any {|l| ($l | str contains 'planner -> exit') and ($l | str contains 'Tracker empty')}) {
        fail 'planner -> exit ("Tracker empty") edge missing: the drained-queue exit belongs to the planner since the tracker guard retired'
    }
    if not ($lines | any {|l| ($l | str contains 'planner -> exit') and ($l | str contains 'Already landed')}) {
        fail 'planner -> exit ("Already landed") edge missing: the planner decision exit lost'
    }

    # 7. closeout files and closes through the capability channel; the
    # evidence node runs --work-item with the packet on stdin
    if not ($code | any {|l| ($l | str contains 'evidence.nu --work-item')}) {
        fail 'evidence script line missing --work-item — the capture must read the granted-item packet mode'
    }
    let cout_start = ($lines | enumerate | where {|e| ($e.item | str trim) == 'closeout ['} | get -o index | first | default null)
    if $cout_start == null { fail 'closeout node not found' }
    let cout_end = ($lines | enumerate | where {|e| $e.index > $cout_start and ($e.item | str trim) == ']'} | get -o index | first | default null)
    if $cout_end == null { fail 'closeout node block not terminated' }
    let cout_block = ($lines | skip ($cout_start + 1) | take ($cout_end - $cout_start - 1))
    if not ($cout_block | any {|l| ($l | str contains 'closeout.nu --work-item')}) {
        fail 'closeout node must run closeout.nu --work-item — the deterministic channel form (denkhaus/work#990)'
    }
    if not ($cout_block | any {|l| ($l | str trim | str trim -r -c ',') == 'stdin_source="current_item_json"'}) {
        fail 'closeout stdin_source lost current_item_json — the packet must reach the close node too'
    }
    if ($cout_block | any {|l| ($l | str contains 'prompt=')}) {
        fail 'closeout must NOT be an agent node anymore — the channel carries the tracker calls (denkhaus/work#990)'
    }

    # 7. gate-red bounce edge is EXPLICIT conditional (work#1029): an
    # unconditional bounce makes the b00c fake-green fold downgrade a
    # completed run to failed/workflow_error while the engine said
    # success (run 01M5ZEWKAMMZ: status split, publish path skipped,
    # work salvaged by hand). The generation cap (work#1057) keeps the
    # 3-reds deadlock guard reachable: without it the bounce conditional
    # outranks the unconditional fallback and swallows the third red too
    # (the fabro-6baf regression).
    # One binding, one truth: the condition literal serves the predicate
    # and the fail message (a copy drifted once already — the review
    # caught >= 3 prose beside a < 2 condition, work#1057).
    let gate_red_cond = 'condition="outcome=failed && nodes.tester.generation < 2"'
    if not ($lines | any {|l| ($l | str contains 'tester -> implementer') and ($l | str contains 'Gate red') and ($l | str contains $gate_red_cond)}) {
        fail $"tester -> implementer Gate red must carry ($gate_red_cond) — an unconditional bounce re-splits engine success from the platform record (work#1029), and a bounce without the generation cap retires the 3-reds human guard (work#1057)"
    }
    # The fallback's own contracts (work#1057): EXACTLY one tester -> exit
    # edge (a second plain route breaks fabro-843d sole_exit_route and
    # leaves the deadlock finish unclassified — probe-30 drive 2), it
    # carries x.kind="deadlock", and it stays UNCONDITIONAL (the third
    # red and unroutable outcomes fall through to it).
    let tester_exits = ($lines | where {|l| ($l | str contains 'tester -> exit')})
    if (($tester_exits | length) != 1) {
        fail $"tester must have exactly ONE edge to exit, found ($tester_exits | length) — a second plain exit route breaks the sole-exit-route classification; probe-30 drive 2 landed succeeded, work#1057"
    }
    let exit_line = ($tester_exits | first)
    if not ($exit_line | str contains 'x.kind="deadlock"') {
        fail 'the tester -> exit fallback must carry x.kind="deadlock" — without it the third red exits unclassified (work#1057)'
    }
    if ($exit_line | str contains 'condition=') {
        fail 'the tester deadlock fallback must stay UNCONDITIONAL — a condition on it would leave the third red and unroutable outcomes with no edge (work#1057)'
    }

    print 'graph-contract-smoke: OK — planner ungated+tool-claiming, preflight report-only on the granted item, retired nodes absent, placement guard wired, closeout files and closes through the channel'
}
