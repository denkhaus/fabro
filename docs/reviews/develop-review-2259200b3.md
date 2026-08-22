<!-- reviewed commit: denkhaus-lab@2259200b3 (2026-08-22) -->

# reviewer-agent report — fabro

> Reviewed: **2026-08-22** · `denkhaus-lab@2259200b3`

Scope: `develop` · allowed tools: `just, ml, nu, nushell, sd` · platform: `fabro, git`

**1 errors · 0 warnings · 1 info**

### [ERROR] AGNOS-02 — workflow asset leaks the current project name 'fabro'
`.fabro/workflows/develop/scripts/evidence.nu:63`
Context: ...et subject_mark = $"fabro\(($run_id)\):"...
**Suggestion:** Replace with a generic placeholder (e.g. `<seed id>`, `the project`).

### [INFO ] GRAPH-04 — genuine failures of 'reviewer' exit the run as success (fail-open)
`.fabro/workflows/develop/workflow.fabro:109`
Edge 'Reviewer failed' (reviewer -> exit) catches outcome=failed. After output_retries are exhausted, malformed JSON or provider errors also end as failed (docs/fabro/failures.md retry layers) — the run then reaches exit and is reported successful. goal_gate=true is NOT an option for nodes that a normal run may never visit: unvisited goal-gate nodes fail the whole run at exit (engine semantics).
**Suggestion:** Acceptable when the tracker keeps the truth (failed seed stays open, the next run re-enters the cycle) — document that trade-off at the edge. Alternatively route genuine failures to an always-visited node that can distinguish them via context flags.

## Verified (pass)

- `.fabro/workflows` tool contract (just, ml, nu, nushell, sd) is provisioned (mise/bootstrap)
- `.fabro/workflows/develop/prompts/implementer.md` prompt of 'implementer' guards the {{ goal }} block against injection
- `.fabro/workflows/develop/prompts/planner.md` prompt of 'planner' guards the {{ goal }} block against injection
- `.fabro/workflows/develop/prompts/reviewer.md` prompt of 'reviewer' guards the {{ goal }} block against injection
- `.fabro/workflows/develop/scripts/evidence.nu:15` checkpoint base detection is scoped to this run (run branch name)
- `.fabro/workflows/develop/workflow.fabro:13` routing contract of 'planner' is consistent (2 labels match edges)
- `.fabro/workflows/develop/workflow.fabro:23` routing contract of 'implementer' is consistent (2 labels match edges)
- `.fabro/workflows/develop/workflow.fabro:71` routing contract of 'reviewer' is consistent (2 labels match edges)
- `.fabro/workflows/develop/workflow.fabro:94` failure exit edge of 'planner' is guarded by goal_gate=true — A genuine failure reaching exit fails the goal gate and thus the run (docs/fabro/outcomes.md L108-119) — correct pattern.
