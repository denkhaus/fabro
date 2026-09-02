# Revision — run 01M0TXPX09279433MGWSWPVVP4

- status reviewed: succeeded
- review: `.fabro/reviews/develop/01M0TXPX09279433MGWSWPVVP4.md`
- seeds filed:
  - fabro-1e9f — Diff evidence against the last seed boundary, not the run base
  - fabro-5870 — Clear transient context keys in closeout
  - fabro-2e66 — Surface PR pipeline degradation instead of succeeding silently
  - fabro-b809 — Make the painpoint channel mandatory-output
  - fabro-ad1b — Cite acceptance bullets in implementer verification, don't restate them
  - fabro-1d94 — Enforce one edit per file path per tool batch
  - fabro-e4fa — Tighten planner sd call economy

## Findings

### Diff evidence against the last seed boundary, not the run base
- filed: fabro-1e9f (priority 1)
- Change: diff against the commit the previous `closeout` produced in `.fabro/workflows/develop/scripts/evidence.nu`, record that SHA as the per-seed base, raise `preamble_budget_kb` in `.fabro/workflows/develop/workflow.fabro` to ~48.
- Expected effect: constant ~18 KB per-seed captures judged inline, zero blob detours, structurally removing preview-only review.

### Keep only the latest visit per non-ignored stage in preambles
- filed: none — duplicate of open seed fabro-699f ("Stop re-rendering every past cycle in preambles"), which covers the same per-cycle preamble growth (input 14,735 → 25,693 tokens, +75% cost from $0.030 to $0.051).

### Clear transient context keys in closeout
- filed: fabro-5870 (priority 2)
- Change: reset `review_verdict`, `review_feedback`, `implementation_summary` via context updates in `.fabro/workflows/develop/scripts/closeout.nu`; delete the stale-verdict compensation paragraph from `.fabro/workflows/develop/prompts/planner.md`.
- Expected effect: stale-verdict ambiguity (seed 1's `approved` surviving into seed-2 stages) eliminated structurally instead of by prose guard.

### Surface PR pipeline degradation instead of succeeding silently
- filed: fabro-2e66 (priority 2)
- Change: force JSON output for PR title/body generation with one repair retry; preflight auto-merge when `auto_merge=true`; degrade terminal report to `succeeded with degraded delivery`.
- Expected effect: no silent gap between run success and an unmerged skeleton PR.

### Make the painpoint channel mandatory-output
- filed: fabro-b809 (priority 2)
- Change: require a `painpoints` array (possibly empty) in outcome JSON of `.fabro/workflows/develop/prompts/planner.md`, `implementer.md`, and `reviewer.md`.
- Expected effect: friction lands in `.fabro/journal/` on the run where it happened, feeding the improve loop with evidence.

### Cite acceptance bullets in implementer verification, don't restate them
- filed: fabro-ad1b (priority 2)
- Change: require `PASS (n)` bullet references instead of quoted criteria, cap prose before JSON at 3 sentences in `.fabro/workflows/develop/prompts/implementer.md`.
- Expected effect: ~1.5–2.5k fewer output tokens per pass, ~$0.05–0.08 and 30–60 s saved per seed, smaller reviewer preambles.

### Enforce one edit per file path per tool batch
- filed: fabro-1d94 (priority 2)
- Change: one line in `.fabro/workflows/develop/prompts/implementer.md` — at most one edit per file path per tool batch; sequence dependent edits across batches.
- Expected effect: eliminates 3 observed concurrent-write serialization warnings and the silent-ordering hazard they signal.

### Tighten planner sd call economy
- filed: fabro-e4fa (priority 2)
- Change: conditional `sd list` rule and `sd ready --format json` in the command table of `.fabro/workflows/develop/prompts/planner.md`.
- Expected effect: one fewer redundant tracker call (byte-identical ~3.3 KB payloads observed) and ~10 s less deliberation per cycle.
