# Revision — run 01M235V6FWMNBDKS9BH3T25G2D

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M235V6FWMNBDKS9BH3T25G2D.md
- seeds filed: fabro-c0ca — Planner briefs: specify insertion points as quoted anchor text, not absolute line numbers
- seeds filed: fabro-8d81 — ml record: print the mx-id on success (upstream) and document an ml search recovery line in implementer.md
- basis: run 01M235V6FWMNBDKS9BH3T25G2D, workflow version 78e816b2619802a23f8ddde5c31eb134267b0d6e9f40ddd161e8c50ff9a697a3, commit 5858b03b0d83e91d34e0425b25c53c9580e18a6c
- revised_at_commit: 5858b03b0d83e91d34e0425b25c53c9580e18a6c (ADR-0015: engine drift signal for later judgement)

## Findings

### Planner briefs: specify insertion points as quoted anchor text, not absolute line numbers

- filed: fabro-c0ca
- Change: in `planner.md` steps 6-7, briefs must name insertion points as quoted anchor text instead of absolute line ranges; drop the planner's anchor-locating greps. Expected effect: -1 implementer verification round and -2 planner tool calls (~35s); eliminates the off-by-N insertion risk class. Distinct from fabro-d20f and fabro-645d (cross-referenced, no supersession).

### ml record: print the mx-id on success (upstream) and document an ml search recovery line in implementer.md

- filed: fabro-8d81
- Change: upstream mulch-cli prints mx-id on successful `ml record`; interim implementer.md line documents recovery via `ml search <domain>` without piping through `head`. Expected effect: -3 tool calls / -2 LLM rounds (~25s, ~$0.015) per lesson-capturing pass. Orthogonal to fabro-96bd and fabro-b94d.
