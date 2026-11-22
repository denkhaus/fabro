#!/usr/bin/env nu
# Graph-contract smoke (fabro-83df / fabro-92e2, 2026-09-19; rewritten for
# the ADR-0032 phase-3 topology): pins the develop graph's deterministic
# contracts. Pure file assertions, no tracker, no git — the same
# reporter-smoke pattern (checked into the repo, wired into
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
#      post-claim preflight). The channel-capable lane_check is the
#      sanctioned resurrection (work#1144): a command stage reading the
#      lane through the run channel — the capability the host script
#      lacked.
#   4. start -> env_guard -> planner wiring: the placement guard is
#      first and start has NO second outgoing edge (fabro-2357). The
#      lane_check node is TEMPORARILY REVERTED (work#1156); the graph
#      pins the reverted shape and the node returns with the fix.
#      The guard node carries on_failure="exit" (work#1277): the
#      default `route` policy let a red guard take its unconditional
#      edge into the planner (sprint-85 misfire, run killed manually);
#      `exit` ends the run failed AT the guard instead.
#   5. The claim-contract chain: planner -> preflight ("Seed claimed"),
#      preflight reading the granted-item packet on stdin
#      (stdin_source="current_item_json", script line carries
#      --work-item), preflight -> implementer ("Preflight done"), and the
#      outcome=failed bounce back to the planner (fabro-c42f class).
#   6. planner -> exit ("Tracker empty") and ("Stale basis") edges
#      EXIST — the drained-queue and superseded-close exits are the
#      planner's own since the tracker guard retired. The already-landed
#      verdict PARKS instead (work#1419): planner -> already_landed_park
#      ("Already landed"), and the park node's sole unconditional
#      x.kind="deadlock" exit — see 6b.
#   7. The work-item tool grants: the planner's x.fabro_tools names
#      fabro_work_next (the atomic claim), the reporter node names
#      fabro_work_close, and the evidence node runs --work-item with the
#      packet on stdin (fork 7 context flow).
#   9. The failed-exit claim hygiene (denkhaus/work#1545): the planner
#      prompt carries the title-retry discipline and the
#      release-before-failed-exit rule — a failed planner never strands
#      its claim.
#   10. The pull reviewer (work#1624, ADR-0051): the reviewer's
#      x.fabro_tools names fabro_run_diffs (the discovery door the pull
#      protocol drives) and keeps fabro_blob (patch paging); the graph
#      carries NO x.fs_hide anywhere (the read hide is retired — reads
#      are free, the checkpoint quarantine owns delivery) while the
#      reviewer's write side stays the empty fs_write capability
#      denial; the reviewer prompt names the pull doors and no longer
#      re-teaches the read-only recital (ADR-0051 deletion contract —
#      the graph attributes are the mechanical owner).

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

    # 8. #1303: the reviewer's patch-paging door — patch content arrives
    # as blob refs and fabro_blob pages it back; without the grant the
    # reviewer dies at the all-error-round limit
    # (run 01M7BAS8MN0FWAWCHXE73K1Z5R).
    # 8b. work#1624 / ADR-0051: the pull-model discovery door — the
    # evidence stage emits metadata only, so the reviewer ENUMERATES the
    # run's checkpoint diffs itself through fabro_run_diffs. The engine
    # tool lands on the parallel branch; fabro-43cf logs a
    # catalog-missing name as a warning until then, never a refusal —
    # the assets lead, the binary follows in the same deploy window.
    let r_start = ($lines | enumerate | where {|e| ($e.item | str trim) == 'reviewer ['} | get -o index | first | default null)
    if $r_start == null { fail 'reviewer node not found' }
    let r_end = ($lines | enumerate | where {|e| $e.index > $r_start and ($e.item | str trim) == ']'} | get -o index | first | default null)
    if $r_end == null { fail 'reviewer node block not terminated' }
    let r_block = ($lines | skip ($r_start + 1) | take ($r_end - $r_start - 1))
    if not ($r_block | any {|l| ($l | str contains 'fabro_blob')}) {
        fail 'reviewer x.fabro_tools missing fabro_blob (#1303) — patch content arrives as blob refs and fabro_blob pages it; without the grant every large review dies'
    }
    if not ($r_block | any {|l| ($l | str contains 'fabro_run_diffs')}) {
        fail 'reviewer x.fabro_tools missing fabro_run_diffs (work#1624, ADR-0051) — the pull model enumerates checkpoint diffs through it; without the grant the reviewer cannot pull'
    }
    # 10b. the reviewer's write side stays a capability denial
    # (fabro-1dae): an empty fs_write denies every file-tool write.
    # Reads free, writes none (ADR-0051).
    if not ($r_block | any {|l| ($l | str trim | str trim -r -c ',') == 'x.fs_write=""'}) {
        fail 'reviewer x.fs_write="" lost — the empty write envelope is the mechanical owner of the write denial (fabro-1dae; ADR-0051 keeps it)'
    }
    # 10c. the reviewer prompt carries the pull protocol and has dropped
    # the read-only recital (ADR-0051 deletion contract: a prompt rule
    # that re-teaches a mechanical enforcement is deleted with the
    # mechanism — the graph attributes above are the owner).
    let REVIEWER_PROMPT = ('.fabro/workflows/develop/prompts/reviewer.md' | path expand)
    if not ($REVIEWER_PROMPT | path exists) { fail 'reviewer prompt not found' }
    let rev_text = (open --raw $REVIEWER_PROMPT)
    if not ($rev_text | str contains 'fabro_run_diffs') {
        fail 'reviewer prompt lost the pull door — the protocol must name fabro_run_diffs (work#1624, ADR-0051)'
    }
    if not ($rev_text | str contains 'fabro_blob') {
        fail 'reviewer prompt lost the patch-paging door — the protocol must name fabro_blob (work#1624, ADR-0051)'
    }
    if ($rev_text | str contains 'BY ENGINE ENFORCEMENT') {
        fail 'reviewer prompt re-teaches the read-only recital (ADR-0051 deletion contract) — the x.tools/x.fs_write graph attributes are the mechanical owner'
    }

    # 2. report-only preflight never exits the run
    if ($lines | any {|l| ($l | str contains 'preflight -> exit')}) {
        fail 'preflight -> exit edge exists (fabro-83df regression): report-only preflight must never exit the run'
    }

    # 3. retired nodes stay retired (comment-STRIPPED lines only — the
    # rationale comments name them by design)
    let code = ($lines | each {|l| ($l | split row '//' | first)})
    # 10a. ADR-0051 read-hide retirement: NO develop node hides reads.
    # The read-hide friction and the shell-bypass teaching die with the
    # attribute; the checkpoint quarantine owns the delivery side.
    # (Pinned over $code — comment lines name the history.)
    if ($code | any {|l| ($l | str contains 'x.fs_hide')}) {
        fail 'x.fs_hide survived in the develop graph (ADR-0051): the read hide is retired — reads are free; the checkpoint quarantine owns delivery'
    }
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
    # work#1277: the guard's red must STOP the run, never route on. The
    # env_guard node block must carry on_failure="exit" (petri Policy::Exit
    # skips the unconditional edge); the default `route` policy routed a
    # red guard straight into the planner in sprint 85.
    # One binding, one truth: the literal serves the predicate and the
    # fail message (the work#1057 rule — a hand-copied pair drifts).
    let guard_policy = 'on_failure="exit"'
    let guard_start = ($lines | enumerate | where {|e| ($e.item | str trim) == 'env_guard ['} | get -o index | first | default null)
    if $guard_start == null { fail 'env_guard node not found' }
    let guard_end = ($lines | enumerate | where {|e| $e.index > $guard_start and ($e.item | str trim) == ']'} | get -o index | first | default null)
    if $guard_end == null { fail 'env_guard node block not terminated' }
    let guard_block = ($lines | skip ($guard_start + 1) | take ($guard_end - $guard_start - 1))
    if not ($guard_block | any {|l| ($l | str trim | str trim -r -c ',') == $guard_policy}) {
        fail $"env_guard missing ($guard_policy) (work#1277): the default route policy lets a red guard take its unconditional edge into the planner — the guard red must end the run"
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
    if not ($lines | any {|l| ($l | str contains 'planner -> exit') and ($l | str contains 'Stale basis')}) {
        fail 'planner -> exit ("Stale basis") edge missing: the stale-basis note exit (ADR-0039 stage G, #1115) lost'
    }
    # 6b. the already-landed park (work#1419, operator decision
    # 2026-11-17): the verdict parks the run needs-human with the claim
    # pinned. The planner routes "Already landed" to the park node — NOT
    # to exit: a x.kind on a planner edge can never fire, because the
    # classifier (fabro-843d sole_exit_route) skips a kind edge beside
    # the planner's plain exit siblings ("Tracker empty", "Stale basis",
    # "Planner failed"), and the old plain edge concluded green PR-less,
    # the adjudicator refused it, and the claim released every lap (a p1
    # item re-claimed each lap at the queue head). The park node's exit
    # contracts mirror the tester fallback's (work#1057): EXACTLY one
    # exit edge (a second plain route breaks the sole-exit-route
    # classification), it carries x.kind="deadlock", and it stays
    # UNCONDITIONAL (the park stage always succeeds).
    if not ($lines | any {|l| ($l | str contains 'planner -> already_landed_park') and ($l | str contains 'Already landed') and ($l | str contains 'preferred_label=\"Already landed\"')}) {
        fail 'planner -> already_landed_park ("Already landed") edge missing: the already-landed verdict must reach the park node (work#1419)'
    }
    let park_exits = ($lines | where {|l| ($l | str contains 'already_landed_park -> exit')})
    if (($park_exits | length) != 1) {
        fail $"the park node must have exactly ONE edge to exit, found ($park_exits | length) — a second plain route breaks the sole-exit-route classification \(fabro-843d\) and the already-landed park would not fire \(work#1419\)"
    }
    let park_exit_line = ($park_exits | first)
    if not ($park_exit_line | str contains 'x.kind="deadlock"') {
        fail 'the park -> exit edge must carry x.kind="deadlock" — without it the already-landed finish exits unclassified, the claim releases, and the item re-enters the queue every lap (work#1419)'
    }
    if ($park_exit_line | str contains 'condition=') {
        fail 'the park -> exit edge must stay UNCONDITIONAL — the park stage always succeeds, and a plain fallback beside the kind edge would break the sole-exit-route classification (fabro-843d)'
    }

    # 7. the reporter records facts through the capability channel; the
    # evidence node runs --work-item with the packet on stdin
    if not ($code | any {|l| ($l | str contains 'evidence.nu --work-item')}) {
        fail 'evidence script line missing --work-item — the capture must read the granted-item packet mode'
    }
    let rep_start = ($lines | enumerate | where {|e| ($e.item | str trim) == 'reporter ['} | get -o index | first | default null)
    if $rep_start == null { fail 'reporter node not found' }
    let rep_end = ($lines | enumerate | where {|e| $e.index > $rep_start and ($e.item | str trim) == ']'} | get -o index | first | default null)
    if $rep_end == null { fail 'reporter node block not terminated' }
    let rep_block = ($lines | skip ($rep_start + 1) | take ($rep_end - $rep_start - 1))
    if not ($rep_block | any {|l| ($l | str contains 'reporter.nu --work-item')}) {
        fail 'reporter node must run reporter.nu --work-item — the deterministic channel form (denkhaus/work#990)'
    }
    if not ($rep_block | any {|l| ($l | str trim | str trim -r -c ',') == 'stdin_source="current_item_json"'}) {
        fail 'reporter stdin_source lost current_item_json — the packet must reach the facts stage too'
    }
    if ($rep_block | any {|l| ($l | str contains 'prompt=')}) {
        fail 'reporter must NOT be an agent node — the channel carries the observation calls (denkhaus/work#990)'
    }
    # The stage-G strip (ADR-0039, #1115): the planner holds NO close
    # grant and no closeout node exists — the adjudicator is the only
    # closer. (Pinned over $code — comment lines name the history.)
    if ($code | any {|l| ($l | str contains 'fabro_work_close')}) {
        fail 'fabro_work_close grant in the develop graph — a run never closes items (ADR-0039 stage G, #1115)'
    }
    if ($code | any {|l| ($l | str contains 'closeout')}) {
        fail 'a closeout reference survived the stage-G rename — the node is the reporter now'
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

    # 9. the failed-exit claim hygiene (denkhaus/work#1545): a planner
    # that claimed and then failed must not strand the claim. The
    # prompt carries the two teachings the incident demanded — the
    # title-retry discipline (a transient fabro_run_set_title failure is
    # re-issued, never counted fatal on its own) and the
    # release-before-failed-exit rule (a genuine planner error releases
    # the claim through fabro_work_release BEFORE emitting the failed
    # JSON; the cycle-guard Blocked route keeps its pin for the human).
    let PLANNER_PROMPT = ('.fabro/workflows/develop/prompts/planner.md' | path expand)
    if not ($PLANNER_PROMPT | path exists) { fail 'planner prompt not found' }
    let prompt_text = (open --raw $PLANNER_PROMPT)
    if not ($prompt_text | str contains 'TITLE-RETRY DISCIPLINE') {
        fail 'planner prompt lost the title-retry discipline (denkhaus/work#1545): a transient set-title failure must be retried, never counted fatal alone — the incident sank a stage whose fourth attempt succeeded'
    }
    if not ($prompt_text | str contains 'RELEASE-BEFORE-FAILED-EXIT') {
        fail 'planner prompt lost the release-before-failed-exit rule (denkhaus/work#1545): a genuine planner error must release the held claim before emitting the failed JSON, or the item strands behind a succeeded run'
    }
    if not ($prompt_text | str contains 'fabro_work_release') {
        fail 'planner prompt names no release tool — the release-before-failed-exit rule is unexecutable without fabro_work_release'
    }

    print 'graph-contract-smoke: OK — planner ungated+tool-claiming, preflight report-only on the granted item, retired nodes absent, placement guard wired, the reporter records facts through the channel, no close authority, failed exits release their claim, the pull reviewer grants fabro_run_diffs+fabro_blob with no fs_hide anywhere and the write denial intact (work#1624, ADR-0051)'
}
