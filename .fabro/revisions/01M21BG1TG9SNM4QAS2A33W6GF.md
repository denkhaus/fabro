# Revision — run 01M21BG1TG9SNM4QAS2A33W6GF

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M21BG1TG9SNM4QAS2A33W6GF.md
- seeds filed: fabro-8bf4 (allow-keys completion lint), fabro-9fa2 (planner stale-basis fast path), fabro-1409 (PR postlude cost table)
- basis: run 01M21BG1TG9SNM4QAS2A33W6GF, workflow version 84429d0a3253eaa52e2406a04a3b97b4365d0ff68b0cdc2f46c296e8c05ec7a6, commit eed7be36252face1b5316bcd9264627fb3da7fde
- revised_at_commit: eed7be36252face1b5316bcd9264627fb3da7fde (ADR-0015: engine drift signal for later judgement)

## Findings

### Engine: lint stage completion when a key declared in context_allow_keys was never emitted
- filed: fabro-8bf4
- Add a completion lint to the stage-envelope validation in `lib/components/fabro-workflow` (alongside the fabro-900e/e47c producer lints): a stage completing without emitting any key declared in its `context_allow_keys` emits a visible warning or consumes one `output_retries` nudge. Scoped as the general allow-keys visibility lint; journal-specific output-schema enforcement already tracked by fabro-017f (cross-referenced, not superseded). Expected effect: missing journal reports and fail-open guards become auditable instead of silently lost.

### Develop planner prompt: stale-basis fast path when the seed pins line anchors in its USER DECISION
- filed: fabro-9fa2
- In `.fabro/workflows/develop/prompts/planner.md` step 3, when the seed body carries a line-pinned USER DECISION or verified basis, one spot-check grep suffices instead of full re-derivation. Complementary to fabro-645d (implementer-side scoped reads). Expected effect: planner drops from ~13 calls/98s toward ~5 calls/40s (~$0.05 + ~1 min per run) and lowers the risk of wrongly closing a valid seed as superseded.

### PR postlude: append per-stage cost/time table and journal painpoint digest to the PR body
- filed: fabro-1409
- Where the engine composes the PR postlude (same area fabro-6a5a targets), append the per-stage timing/cost table plus a digest of journal `painpoints`. Degraded-guard notices excluded — already tracked by fabro-5b0a. Expected effect: at merge time the user sees that builds dominate (the case for toolchain warming, fabro-fe15) without opening the run dashboard.
