# Revision — run 01M2QWF2RX4Q0BTB8AQWPH0K4M

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2QWF2RX4Q0BTB8AQWPH0K4M.md
- seeds filed: fabro-fb06 — Pre-claim green-probe routing: execute the brief's cheapest repro before claiming; green takes the Verification-only edge
- basis: run 01M2QWF2RX4Q0BTB8AQWPH0K4M, workflow version f4ab9ff48b6e162ee81f42689c29e27253fba4fe3632992c2e164afa31eb5ca5, commit 6d2b59ef78916a7667cecb3a1f06bb056718bd88
- revised_at_commit: 6d2b59ef78916a7667cecb3a1f06bb056718bd88 (ADR-0015: engine drift signal for later judgement)

## Findings

### Pre-claim green-probe routing (filed as fabro-fb06)

Planner verified fabro-ca1f only by `rg -l` on test names; the 12-test battery was already green on the untouched tree. The implementer then burned 633 s wall (86% of run, $0.132 / 50% of cost) on cold compiles and full crate suites to rediscover green, and tester/evidence still ran on an empty diff. Change: `.fabro/workflows/develop/prompts/planner.md` step 3 plus `.fabro/workflows/develop/scripts/planner-preflight.nu` should execute the cheapest named repro for bug seeds before `sd update ... in_progress`; a green result deterministically takes the existing Verification-only edge (fabro-9d26) instead of Seed claimed. Expected effect: this run class finishes in ~3 min instead of 12.6. No duplicate: fabro-9d26 (LLM-judgment edge), fabro-4be6 (step-7 dry-runs), fabro-a32f (landed PRs) each cover different triggers. Not capability-affecting (prompt/script change, no tool/credential delta).
