The develop child could not be created: `fabro_run_create` returns `workflow not found: develop` on every variant (slug, graph name `Develop`, file path with/without `workflow.toml`, version id, cwd overrides `/workspace/fabro` and `/repos/denkhaus/fabro`, explicit `start`, parent-scoped). Meanwhile `fabro_runs_list` for workflow `develop` succeeds and shows the workflow registered on the server (`workflow_slug: "develop"`, 46 runs, latest `01M1RJK3...` succeeded), and `.fabro/workflows/conductor/workflow.fabro` explicitly grants `fabro_run_create` with `inspects="develop,merge-upstream,revisor"`. So the workflow exists and is inspectable, but the create path cannot resolve it — a create/registry resolution failure in the tooling, not a missing workflow.

Per the outcome contract this is the `failed` route (create/poll tooling failed). No child run exists; nothing was mutated.

```json
{
  "outcome": "failed",
  "failure_reason": "fabro_run_create cannot resolve workflow 'develop' ('workflow not found: develop') on any selector form (slug, graph name, .fabro/workflows/develop/workflow.toml path, version id, cwd overrides, explicit start, parent-scoped), while fabro_runs_list proves the workflow is registered and inspectable (workflow_slug 'develop', 46 runs; conductor graph declares inspects='develop,merge-upstream,revisor' and grants fabro_run_create). Create-path workflow resolution is broken; no child run was created and nothing was mutated.",
  "preferred_next_label": "Conductor pass ends (tooling failure)",
  "suggested_next_ids": [],
  "context_updates": {
    "develop_leg": "failed: fabro_run_create workflow resolution error; retry next conductor fire after fixing create-path lookup",
    "attempts": "7 create variants, all 'workflow not found': develop, Develop, .fabro/workflows/develop/workflow.toml, .fabro/workflows/develop, version id 2831136c..., with cwd /workspace/fabro and /repos/denkhaus/fabro, with start=true, parent-scoped to 01M1RQGZYTPZH1RKRSTKXZZWN4"
  }
}
```