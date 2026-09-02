# Revision — run 01M0SS23MJM8972CBJD0SP7T4Q

- status reviewed: failed
- review: .fabro/reviews/develop/01M0SS23MJM8972CBJD0SP7T4Q.md
- seeds filed:
  - fabro-d97b — Convert reviewer to agent node with read-only tools
  - fabro-699f — Stop re-rendering every past cycle in preambles
  - fabro-6a77 — Treat branch-pushed-PR-failed as completed-with-publish-warning
  - fabro-fac9 — Cap evidence flowing through context updates in planner and implementer prompts
  - fabro-8354 — Bake ripgrep into the runner image
  - fabro-850f — Write only non-empty stage journals
  - fabro-c2ca — Skip metadata snapshots on non-agent stages

## Findings

1. Fix reviewer evidence pipe — filed as fabro-d97b (agent-node half; the preamble-budget half is duplicate-of fabro-83f9). Convert the reviewer to an agent node with read-only tools so blob-ref'd evidence is readable; expected effect: no more blind rejections/approvals — the workaround cycle in this run cost $0.112 of $0.257 (43%) for zero code change.

2. Stop re-rendering every past cycle in preambles — filed as fabro-699f. Render only the latest visit of command stages and clear consumed context keys after the planner handles them; expected effect: preamble growth stops at cycle 3+ instead of blowing the 12KB budget before max_visits=20.

3. Treat branch-pushed-PR-failed as completed-with-publish-warning — filed as fabro-6a77. Pre-run PR-scope credential check in prepare plus a retryable publish-warning downgrade and fixed PR-body JSON contract; expected effect: run status reflects loop outcome — this run closed seed `fabro-e6df` and pushed 13 commits yet reads as failed.

4. Add Evidence-unreadable reviewer verdict — duplicate-of fabro-886e (Add Evidence retry verdict to reviewer); no new seed filed.

5. Align runner dockerfile mise versions — duplicate-of fabro-a54c and fabro-80ec; no new seed filed.

6. Cap evidence in context updates — filed as fabro-fac9. Cap summaries at ~500 chars and drop the inline-report workaround once the evidence pipe is fixed; expected effect: stops the 2.6KB inlined `implementation_summary` that itself blob-offloaded.

7. Bake ripgrep into the runner image — filed as fabro-8354. One dockerfile line; expected effect: removes ~5 wasted exploration calls per planner pass caused by missing `rg`.

8. Write only non-empty stage journals — filed as fabro-850f. Populate or skip empty `data` in `scripts/stage-journal.nu`; expected effect: stops 13 identical `{\"data\": {}}` files per run from inflating checkpoint churn.

9. Skip metadata snapshots on non-agent stages — filed as fabro-c2ca. Skip snapshots for tester/evidence stages; expected effect: ~25s wall-time saving per run with no evidence loss.

10. Scope skill discovery off — duplicate-of fabro-2c81 (Scope skill discovery to workflow skills dir); no new seed filed.
