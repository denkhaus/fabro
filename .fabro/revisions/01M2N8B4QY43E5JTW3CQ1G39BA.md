# Revision — run 01M2N8B4QY43E5JTW3CQ1G39BA

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2N8B4QY43E5JTW3CQ1G39BA.md
- seeds filed: fabro-4b76 (adopt `--self <run-id>` in dup-run-check callers + checked-in fixture battery), fabro-cf2a (stamp Fabro-Run trailers on closeout seed-sync commits), fabro-ae74 (structured arms list in multi-appendage seeds)
- basis: run 01M2N8B4QY43E5JTW3CQ1G39BA, workflow version e1bda2f455a07c0d501ab040c95e4344d05b2f486e5a471db6e4c56668ad1c61, commit 8a646048cc516f870e7472f5fd9a31c06f2d661b
- revised_at_commit: 8a646048cc516f870e7472f5fd9a31c06f2d661b (ADR-0015: engine drift signal for later judgement)

## Findings

### Adopt `--self <run-id>` in dup-run-check callers and check in the fixture battery
- filed: fabro-4b76
- The implementer preflight ran `nu .fabro/scripts/dup-run-check.nu fabro-e6a0` without `--self` (run events seq 81), leaving the closure-identity feature this very run shipped (fabro-e6a0, PR #185) dead at its call sites. Fix: implementer step 1 and revisor analyze Step 3.5 pass `--self <run-id>`; verification battery uses a checked-in fixture script instead of improvised `/tmp` sd wrappers. Expected effect: mechanical self/foreign verdicts; per-pass manual trailer-adjudication tax ends.

### Stamp Fabro-Run trailers on closeout seed-sync commits so self-closures classify mechanically
- filed: fabro-cf2a
- dup-run-check on the run's own seed returned `verdict duplicate / closure foreign` — a false positive on a healthy run. The squash subject carries no seed id and the closing seeds-sync commit carries no Fabro-Run trailer, so the seeds-close fallback cannot see self-closure; `--self` alone would not have fixed it. Fix: stamp the trailer on closeout `sd sync` commits via the fabro-checkpoint trailer machinery (`lib/components/fabro-checkpoint`) and/or teach the seeds-close fallback to resolve the adjacent merge's trailer. Expected effect: healthy self-closed runs stop arriving as duplicates (8th manual adjudication this pass).

### Require a structured arms list in multi-appendage seeds
- filed: fabro-ae74
- The planner had to hand-fold fabro-e6a0's 6 dated update appendages (4 adjudication arms) into bullets. Fix: seed-authoring convention (revise intake + `docs/agents/issue-tracker.md`) requiring a structured arms list once a seed gains a second dated occurrence-appendage. Expected effect: brieves stop re-deriving arm structure from prose.
