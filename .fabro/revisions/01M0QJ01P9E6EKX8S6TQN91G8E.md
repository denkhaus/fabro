# Revision — run 01M0QJ01P9E6EKX8S6TQN91G8E

- status reviewed: failed
- review: .fabro/reviews/develop/01M0QJ01P9E6EKX8S6TQN91G8E.md
- seeds filed: fabro-b158 Fix invalid reasoning_effort pin on planner node; fabro-cbca Validate node-level model controls at submit time; fabro-e7e5 Degrade on unsupported model control instead of dead-ending; fabro-0230 Surface root-cause error in terminal run status; fabro-2c81 Scope skill discovery to workflow skills dir; fabro-5632 Skip empty checkpoint commits on failed runs; fabro-80ec Pre-install mise toolset in runner image

## Findings

### Fix invalid reasoning_effort pin on planner node — filed fabro-b158
In `.fabro/workflows/develop/workflow.fabro` (planner node, line 29) change `reasoning_effort="medium"` to `"low"` or remove the node-level pin — glm-5.3 only allows low/high/max. The run died 1.5s into the planner stage with `invalid_request: model 'glm-5.3' does not support reasoning_effort 'medium'` (0 tokens, will_retry=false). Expected effect: the planner's first LLM call succeeds and this exact failure class becomes impossible.

### Validate node-level model controls at submit time — filed fabro-cbca
Cross-check node attrs like `reasoning_effort` against the resolved provider/model capabilities before sandbox creation (sync-check or engine submit path). The error was purely static but was only discovered after 33.4s of sandbox + setup. Expected effect: config-class errors fail in under 1s with the provider's exact message instead of a ~51s run costing a sandbox and a pushed branch.

### Degrade on unsupported model control instead of dead-ending — filed fabro-e7e5
Configure `fallbacks` and/or retry once with the unsupported control dropped on `api_deterministic/invalid_request`. This run had `max_attempts=5` but `will_retry=false` and `fallbacks = {}`, so one parameter mismatch consumed the whole goal-gated run. Expected effect: model swaps or control drift degrade a single call to defaults instead of failing the run.

### Surface root-cause error in terminal run status — filed fabro-0230
Make the terminal failure reason the original signature `planner|…|invalid_request` instead of `goal gate unsatisfied for node planner and no retry target` — via the conclusion mapping or by dropping `goal_gate=true` on the planner node (planner failure already has an explicit edge to refiner). Expected effect: run lists show an actionable root cause in seconds.

### Scope skill discovery to workflow skills dir — filed fabro-2c81
Scope skill source dirs per workflow/node to `.fabro/skills`, dropping `/storage/.home/skills`. The `agent.skills.discovered` event pulled 36 skills into a planner that only needs `sd`, bloating preambles against `preamble_budget_kb=12`. Expected effect: smaller preambles, less role-prompt distraction, and removal of the skill-token crash class the prompts defensively work around.

### Skip empty checkpoint commits on failed runs — filed fabro-5632
Skip commit/push when the stage diff is empty and the run has no retry target. After the planner failed (0 tokens, 0 files changed), the run still spent ~14s on two meta snapshots, two empty commits (`dfb266e`, `d99a357`) and two remote pushes. Expected effect: failed runs terminate within ~2s of their terminal stage with no empty commits to clean up.

### Pre-install mise toolset in runner image — filed fabro-80ec
Pre-install the `.mise.toml` toolset in the `fabro-runner:mise` Dockerfile or mount a mise cache volume. `mise install` took 26.6s of the 28.5s setup for a run whose useful life was 1.5s. Expected effect: ~27s saved per run, mattering most for short or failed runs where setup dominated 2:1.
