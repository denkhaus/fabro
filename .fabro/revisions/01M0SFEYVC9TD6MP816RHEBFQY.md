# Revision — run 01M0SFEYVC9TD6MP816RHEBFQY

- status reviewed: failed
- review: .fabro/reviews/develop/01M0SFEYVC9TD6MP816RHEBFQY.md
- seeds filed: fabro-0da8 — Add a deterministic tracker guard node before the planner; fabro-9b33 — Document sd list read semantics in the planner prompt

## Findings

### Add a deterministic tracker guard node before the planner
- filed: fabro-0da8
- Change: add a script node after `start` in `.fabro/workflows/develop/workflow.fabro` running `sd list` and `sd list --status in_progress`; when both are empty, route Tracker empty directly to exit, else continue to planner.
- Expected effect: the drained-tracker terminal case costs zero agent tokens; planner only runs when there is work.

### Document sd list read semantics in the planner prompt
- filed: fabro-9b33
- Change: extend `.fabro/workflows/develop/prompts/planner.md` tracker mechanics with the read semantics of `sd list` (open-only default, valid `--status` values, no `all`).
- Expected effect: one fewer inference turn per planner invocation; faster correct Tracker-empty routing.
