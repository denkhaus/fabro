# Revision — run 01M2VX6PVR87WS6XJQNY6YMMFE

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2VX6PVR87WS6XJQNY6YMMFE.md
- seeds filed: none — healthy run, zero balance credit (ADR-0022); all findings journaled as overflow
- balance: 0 non-exempt seeds filed / 0 — no credit this pass
- basis: run 01M2VX6PVR87WS6XJQNY6YMMFE, workflow version 341257a3c3ccf500ad9774c1dab92b6aef751603b31e66b83b7c48131841f51b, commit ac7d73f71bad79a6301272e69dd2af1156b5e516
- revised_at_commit: ac7d73f71bad79a6301272e69dd2af1156b5e516 (ADR-0015: engine drift signal for later judgement)

## Findings

All four analyst findings survived triage but this pass has zero same-pass
stale/superseded close credit, so per ADR-0022 nothing is filed; each is
recorded as a named `overflow:` observation in the revisor journal for the
next pass to re-file after re-running its own dedupe.

1. **Park upstream-PR-gated seeds mechanically in the planner preflight** — overflow to journal. Add an `externally_gated` arm to the verdict table in `.fabro/workflows/develop/scripts/planner-preflight.nu` (fold into open fabro-ead4 or file standalone); one `sd update fabro-af22` adding a `Gate: upstream PR <url>` line now. Expected effect: −60s and ~$0.13–0.15 per develop run until upstream PR fabro-sh/fabro#784 merges.
2. **Raise reviewer preamble_output_max_lines 200→400** — overflow to journal (consolidated with finding 4, same file `.fabro/workflows/develop/workflow.fabro`). First post-c9f2 truncation recurrence; this run's reviewer preamble opened with "(59 lines omitted)", forcing one shell rg fallback. Expected effect: full captures inline, wrongful-rejection risk class eliminated.
3. **Add an ack/waiver mechanism to prompt-lint routing-field-schema-warnings** — overflow to journal. In `.fabro/scripts/prompt-lint.nu` check 5, support an `x-routing-intended` marker or waiver list so intentionally-routing schemas stay silent; currently 6 permanent warnings per lint run. Expected effect: lint warnings stay actionable.
4. **Declare output.planner in the planner node's context_allow_keys** — overflow to journal (consolidated into finding 2's multi-arm seed shape, same file). Kills one `context_update_dropped` WARN per planner visit with no behavior change.
