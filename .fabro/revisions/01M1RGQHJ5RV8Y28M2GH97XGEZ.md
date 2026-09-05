# Revision — run 01M1RGQHJ5RV8Y28M2GH97XGEZ

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M1RGQHJ5RV8Y28M2GH97XGEZ.md
- seeds filed: fabro-270c — Make the journal payload shape retry-proof in develop prompts; fabro-a512 — Correct the disproved fs_hide claim in workflow.fabro comments
- basis: run 01M1RGQHJ5RV8Y28M2GH97XGEZ, workflow version 265a7df1ba6776f527c47bd6508e28afe7b4bdbf83922b13cac16f764b0ff586, commit 309e94fb54c039630f1b6ee81e0d754ec99fdc1e
- revised_at_commit: 309e94fb54c039630f1b6ee81e0d754ec99fdc1e (ADR-0015: engine drift signal for later judgement)

## Findings

### Make the journal payload shape retry-proof in develop prompts
- filed: fabro-270c
- The planner's first final answer emitted malformed journal JSON (`["observations" : "none"]`); the routing validator rejected it (seq 91) and the retry burned ~13 s plus a full-context re-read. Make `painpoints` and `observations` both plain string arrays in the Journal sections of the develop prompts (or add engine-side repair of trivial syntax errors). Effect: eliminates a full retry turn per malformed emission.

### Correct the disproved fs_hide claim in workflow.fabro comments
- filed: fabro-a512
- The implementer node comment in `.fabro/workflows/develop/workflow.fabro` still asserts hidden paths are unwritable, and the reviewer (whose node has no fs_hide) believed reads were denied and avoided native file tools (seq 179). Rewrite the comment to state fs_hide binds file tools only, shell succeeds, policy governs the shell, with per-node scope. Effect: docs stop re-teaching the disproved model; reviewer uses cheaper native read tools.
