# Revision — run 01M0QTNF6GWHGPKHSQ2Y2PB8CQ

- status reviewed: failed
- review: .fabro/reviews/develop/01M0QTNF6GWHGPKHSQ2Y2PB8CQ.md
- seeds filed: fabro-cc23 — Projectize prime boilerplate with the real gate in AGENTS.md; fabro-3814 — Tell implementer to skip sd prime when the brief is already in context

## Findings

### Projectize prime boilerplate with the real gate in AGENTS.md
- filed: fabro-cc23
- Change: add a project-override note in `AGENTS.md` outside the tool-generated seeds-onboard markers stating this repo's close protocol is `just qualitygate` and bun-based prime instructions do not apply.
- Expected effect: removes wrong-stack misexecution risk and stops contradictory context in every agent stage (planner@1 and implementer@1 each ingested ~5 KB of generic template demanding nonexistent bun commands).

### Tell implementer to skip sd prime when the brief is already in context
- filed: fabro-3814
- Change: one line in `.fabro/workflows/develop/prompts/implementer.md` to skip `sd prime` when `current_seed_brief` is already in context.
- Expected effect: trims ~5 KB per implementer visit with no information loss (implementer was $0.118 of the run's $0.181, 65%).

### Not filed (already addressed or engine-side)
- Reviewer agent node, `preamble_budget_kb` 24, `summary:high`, toolchain incl. ripgrep in `.fabro/Dockerfile.mise`, gate allowlist in `prompts/implementer.md`, journal bridge fabro-31b2 in `scripts/stage-journal.nu`, `sd ready`-first guidance in `prompts/planner.md` — verified already present in current assets.
- Publish fail-safe (retry + deterministic PR title/body + `republish` control) and progress counter fix (counts stage executions instead of nodes; not refreshed per `checkpoint.completed`) — engine-side, routed via painpoint channel.
