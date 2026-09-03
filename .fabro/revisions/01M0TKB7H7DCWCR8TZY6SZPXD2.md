# Revision — run 01M0TKB7H7DCWCR8TZY6SZPXD2

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M0TKB7H7DCWCR8TZY6SZPXD2.md
- seeds filed: fabro-50c9 Cap reviewer re-verification to capture-only judging; fabro-a67f Whitelist implementer smoke checks and make spec re-fetch conditional; fabro-9ef9 Give the evidence node its own output context key; fabro-89dd Add a review_notes channel for non-blocking findings; fabro-b382 Forbid brief duplication and empty-tracker double checks in planner

## Findings

### Cap reviewer re-verification to capture-only judging
- filed: fabro-50c9 (priority 1)
- Change: add a verification-economy clause to `.fabro/workflows/develop/prompts/reviewer.md` — judge from the evidence capture; tools only for claims the capture cannot show; never re-run a check already pinned in the diff. Expected effect: review cycle ~62s → ~35–40s, ~$0.02/seed.

### Whitelist implementer smoke checks and make spec re-fetch conditional
- filed: fabro-a67f (priority 1; partially overlaps fabro-578a smoke-level commands and fabro-3814 sd-prime skip, but covers the whitelist plus the brief-authority fix)
- Change: hard whitelist (build + affected test package, never `just qualitygate`) and conditional step-1 `sd show` in `.fabro/workflows/develop/prompts/implementer.md`. Expected effect: ~25–40% off implementer inference (~30–45s, ~$0.03/seed); brief-vs-spec authority ambiguity removed.

### Raise preamble_budget_kb from 12 to 24
- filed: none — duplicate of existing fabro-83f9 ("Raise reviewer preamble budget from 12KB to 20-24KB")

### Give the evidence node its own output context key
- filed: fabro-9ef9 (priority 2)
- Change: key evidence node to `evidence.output` via command-node keying in `.fabro/workflows/develop/workflow.fabro` and `scripts/evidence.nu`. Expected effect: no last-writer-wins gate-log clobber; gate result survives a red-gate bounce.

### Add a review_notes channel for non-blocking findings
- filed: fabro-89dd (priority 2)
- Change: optional `context_updates.review_notes` in Approved JSON; planner folds into a brief or parks a polish seed. Expected effect: hygiene findings accumulate instead of evaporating at approval.

### Forbid brief duplication and empty-tracker double checks in planner
- filed: fabro-b382 (priority 2)
- Change: absolute no-brief-repetition closing rule plus decision-order placement for the `sd list` restriction in `.fabro/workflows/develop/prompts/planner.md`. Expected effect: ~1.5KB less carried context per cycle, one turn off terminal planner passes.
