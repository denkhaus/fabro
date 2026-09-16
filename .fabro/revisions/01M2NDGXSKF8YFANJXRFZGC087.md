# Revision — run 01M2NDGXSKF8YFANJXRFZGC087

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2NDGXSKF8YFANJXRFZGC087.md
- seeds filed: fabro-ac84 — Execute checked-in fixture batteries in the qualitygate loop-asset tier
- basis: run 01M2NDGXSKF8YFANJXRFZGC087, workflow version e1bda2f455a07c0d501ab040c95e4344d05b2f486e5a471db6e4c56668ad1c61, commit 70580c1eb7731c2a82c60dae9fd129026de86a13
- revised_at_commit: 70580c1eb7731c2a82c60dae9fd129026de86a13 (ADR-0015: engine drift signal for later judgement)

## Findings

### Execute checked-in fixture batteries in the qualitygate loop-asset tier
- filed: fabro-ac84
- The tester gate lint-checks loop-asset nu scripts but never executes them. Run 01M2NDGXSKF8YFANJXRFZGC087 created `.fabro/scripts/dup-run-check-fixtures.nu` (7/7 PASS) and the reviewer re-ran the battery by hand (reviewer journal, checkpoint seq 248, seed fabro-4b76). Change: extend `check-loop-assets` in `scripts/qualitygate.nu` — already executing the checked-in `evidence-smoke.nu` regression from closed fabro-bfe1 — to also execute checked-in fixture batteries under `.fabro/scripts/` (start with `dup-run-check-fixtures.nu`). Expected effect: regressions surface deterministically at the tester node; reviewers stop re-proving executable acceptance criteria. Duplicate check: no open or closed seed names this concrete change (fabro-50f8, fabro-8a60, fabro-6e7f adjacent; fabro-bfe1 predates and does not cover the battery).
