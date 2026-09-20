# Revision — run 01M294HH117ZMV11HCNCEQPCDJ

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M294HH117ZMV11HCNCEQPCDJ.md
- seeds filed: fabro-189d (evidence node resolves claimed seed via stdin_source=current_seed_id, hard-fail on mismatch); fabro-7a9e (planner PROJECT_FACTS: sd show JSON with no description key IS the empty body); fabro-41b1 (PR postlude PR-body generation non-strict); fabro-7f27 (revisor intake requires body on filed seeds)
- basis: run 01M294HH117ZMV11HCNCEQPCDJ, workflow version e825d8c827ce15defe0aa57d3586b0f369cab48cc7fe711d526776762f442aa1, commit 1caf1ee66bbf5a8e5a73b66446720cf20a3768ea
- revised_at_commit: 1caf1ee66bbf5a8e5a73b66446720cf20a3768ea (ADR-0015: engine drift signal for later judgement)

## Findings

### Evidence node: resolve the claimed seed via stdin_source=current_seed_id, hard-fail on mismatch
- filed: fabro-189d (priority 1)
- Evidence capture quoted the wrong concurrent in_progress seed (fabro-5082) as authoritative and used a stale diff-base; reviewer burned ~1.9k tokens verifying the real seed (event seq 787). Change: stdin_source="current_seed_id" on the evidence node in the workflow graph, and `evidence.nu` resolves spec + claim base from that id, exiting non-zero on mismatch. Complements open fabro-1e9f; closed fabro-4b57 is a different change (duplicate check: searched "evidence seed id stdin", "diff-base").

### Planner PROJECT_FACTS: sd show JSON with no description key IS the empty body
- filed: fabro-7a9e (priority 2)
- Planner burned four sequential calls (seq 37-58) proving an empty body. One-line addition to the PROJECT_FACTS sd-command table in `prompts/planner.md`; ~3 fewer planner turns per title-only seed. Complements fabro-55a7 (duplicate check: searched "PROJECT_FACTS empty description probe", "planner turns").

### PR postlude: make PR-body generation non-strict
- filed: fabro-41b1 (priority 2)
- Worker log shows strict-JSON parse failure + retry on every run's terminal PR-body step. Make the postlude's PR-body model call non-strict or lenient-schema; removes one retry and one warn per run. Closed fabro-6a5a took a different route, partially reverted by fabro-d114 (duplicate check: searched "PR body strict JSON retry", "postlude strict").

### Revisor intake: require a body on filed seeds
- filed: fabro-7f27 (priority 2)
- Title-only seed fabro-3ef7 forced the planner to author acceptance criteria from the title; reviewer judged planner-derived criteria. Intake lint rejecting/flagging body-less seeds plus legacy backfill (duplicate check: searched "intake require body title-only seed", "backfill title-only").
