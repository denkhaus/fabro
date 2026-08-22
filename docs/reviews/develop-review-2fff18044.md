<!-- reviewed commit: denkhaus-lab@2fff18044 (2026-08-22) -->

# reviewer-agent report — fabro-lab

Scope: `develop` · allowed tools: `just, ml, nu, nushell, sd` · platform: `fabro, git`

**0 errors · 1 warnings · 1 info**

### [WARN ] SCRIPT-05 — diff-filter=ACMRT excludes deletions from the per-file diffs
`.fabro/workflows/develop/scripts/evidence.nu:42`
Deleted files appear only in the --stat overview, never in the per-file diff body the reviewer reads. A seed whose acceptance criterion is 'file X is gone' cannot be verified from the evidence.
**Suggestion:** Use --diff-filter=ACMRTD (or drop the filter) so deletions are shown.

### [INFO ] GRAPH-04 — genuine failures of 'reviewer' exit the run as success (fail-open)
`.fabro/workflows/develop/workflow.fabro:84`
Edge 'Reviewer failed' (reviewer -> exit) catches outcome=failed. After output_retries are exhausted, malformed JSON or provider errors also end as failed (docs/fabro/failures.md retry layers) — the run then reaches exit and is reported successful. goal_gate=true is NOT an option for nodes that a normal run may never visit: unvisited goal-gate nodes fail the whole run at exit (engine semantics).
**Suggestion:** Acceptable when the tracker keeps the truth (failed seed stays open, the next run re-enters the cycle) — document that trade-off at the edge. Alternatively route genuine failures to an always-visited node that can distinguish them via context flags.

## Verified (pass)

- `.fabro/workflows` tool contract (just, ml, nu, nushell, sd) is provisioned (mise/bootstrap)
- `.fabro/workflows/develop/prompts/implementer.md` prompt of 'implementer' guards the {{ goal }} block against injection
- `.fabro/workflows/develop/prompts/planner.md` prompt of 'planner' guards the {{ goal }} block against injection
- `.fabro/workflows/develop/prompts/reviewer.md` prompt of 'reviewer' guards the {{ goal }} block against injection
- `.fabro/workflows/develop/prompts/reviewer.md:11` evidence diff budget is disclosed to the reviewer (truncation is explicit)
- `.fabro/workflows/develop/scripts/evidence.nu:7` checkpoint base detection is scoped to this run (run branch name)
- `.fabro/workflows/develop/workflow.fabro:13` routing contract of 'planner' is consistent (2 labels match edges)
- `.fabro/workflows/develop/workflow.fabro:23` routing contract of 'implementer' is consistent (2 labels match edges)
- `.fabro/workflows/develop/workflow.fabro:59` routing contract of 'reviewer' is consistent (2 labels match edges)
- `.fabro/workflows/develop/workflow.fabro:70` failure exit edge of 'planner' is guarded by goal_gate=true — A genuine failure reaching exit fails the goal gate and thus the run (docs/fabro/outcomes.md L108-119) — correct pattern.
- `justfile:11` all just recipes referenced by the workflow exist (qualitygate)
