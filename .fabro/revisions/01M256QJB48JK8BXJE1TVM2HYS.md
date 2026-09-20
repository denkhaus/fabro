# Revision — run 01M256QJB48JK8BXJE1TVM2HYS

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M256QJB48JK8BXJE1TVM2HYS.md
- seeds filed: fabro-d183 (planner exits already-landed seeds before the cycle), fabro-dad8 (probe contradictions written into acceptance bullets as exemptions), fabro-52b4 (read-only PROJECT_FACTS subset for the reviewer include), fabro-b1d3 (Slack run.completed bookkeeping-only annotation)
- basis: run 01M256QJB48JK8BXJE1TVM2HYS, workflow version 38bab0b8b1faad62aaddb5e68facf6f1c58286169676479d3edbf99d4205b91c, commit 4d8a1e7f7a4fd74bed2dad41516106a8969a7e0b
- revised_at_commit: 4d8a1e7f7a4fd74bed2dad41516106a8969a7e0b (ADR-0015: engine drift signal for later judgement)

## Findings

### Planner: exit already-landed seeds before the cycle instead of running a no-op verification lap
- filed: fabro-d183
- Change: in `.fabro/workflows/develop/prompts/planner.md` step 3, make the already-implemented case mechanical — `git log --grep <seed-id>` hit in base history plus satisfied criteria closes via `sd close --reason "superseded: fix landed in <sha>"` and routes exit (needs a planner exit edge); verification-only claim reserved for criteria satisfied with NO referencing commit. Expected effect: eliminates a whole ~$0.16, 3.5-min run plus a bookkeeping-only PR per stale tracker row. Distinct from fabro-5656 and fabro-ab38 (claim-time decision).

### Planner: write probe contradictions into acceptance bullets as explicit exemptions
- filed: fabro-dad8
- Change: one line in `.fabro/workflows/develop/prompts/planner.md` steps 6-7 — when probe output contradicts an acceptance bullet's literal wording, rewrite the bullet with the exemption. Expected effect: removes ambiguity that can flip a strict reviewer into a ~3-minute re-plan cycle (this run's line-89 comment vs "no `get -o` anywhere").

### Reviewer: read-only PROJECT_FACTS subset — drop tracker write commands from the reviewer prompt include
- filed: fabro-52b4
- Change: render a read-only PROJECT_FACTS subset for `.fabro/workflows/develop/prompts/reviewer.md` (no `sd update`/`sd close` write table). Expected effect: fewer reviewer prompt tokens, smaller attack surface for rogue tracker writes. Scope limited to the include content; per-node memory scoping stays in fabro-9588.

### Slack: annotate bookkeeping-only runs in the run.completed payload
- filed: fabro-b1d3
- Change: when the run diff touches only `.fabro/journal/**` and `.seeds/**`, annotate the Slack `run.completed` payload as bookkeeping-only. Expected effect: humans triage no-op runs in Slack instead of opening journal-only PRs. Complements fabro-9f97 (PR side); no supersession — complementary mechanisms.
