# Feature touchpoints to check on every upstream merge

Our fork features that must not regress. For each: name the seed, the
code locations, and the fast verification when the touched area overlaps.

| Feature (seed) | Code locations | Fast verification |
|---|---|---|
| Publish-blocked taxonomy (fabro-67e5, closed) | fabro-types status.rs SuccessReason; workflow pipeline/finalize.rs build_terminal_event; server.rs event application + slack; cli run/wait.rs; web header.tsx | nextest -p fabro-workflow pipeline::finalize + fabro-server publish_blocked |
| Boundary exit kind (fabro-08b4, closed) | Same as above + apply_boundary_upgrade; context.mdx docs | nextest boundary tests in finalize |
| PR create retry (fabro-67e5, closed) | fabro-github CreatePullRequestError; workflow pull_request.rs create_pull_request_with_attempts | nextest -p fabro-github + pipeline::pull_request |
| PR model plumbing (fabro-890b, OPEN) | operations/start.rs resolve_pr_model; persisted run spec | WATCH: upstream moved pr_origin_url nearby — this seed should be fixed ON the merged code |
| spa_refresh mirror race (fabro-332e, OPEN) | fabro-dev spa_refresh.rs; justfile lock; scripts/smoke.nu | just smoke after deploy |
| ask duplication (fabro-bd6c, OPEN) | cli commands/run/ask.rs render_event | manual two-token probe |
| attach replay indistinguishable (fabro-204e, OPEN) | cli attach.rs | attach a finished run |
| Preamble aggregate budget (fabro-a85b, OPEN) | workflow artifact.rs demote_large_values_for_prompt + tests | nextest artifact tests |
| just-up lock + smoke (landed) | justfile, scripts/smoke.nu, scripts/wait-healthy.nu | just up full pipeline |
| run_workflow.nu pipeline (landed) | scripts/run_workflow.nu, scripts/prompts/improve.md | just run hello --adopt <id> |
| Auto-merge wiring (fabro-ab2c, CLOSED via branch protection) | .github/workflows/lab-check.yml; repo settings; run_workflow auto-merge poll | run_workflow integrates via ff-pull |

## Obsolescence watchlist

Upstream directions that may supersede our work — re-evaluate per merge:

- Sandbox runtime directory (`/tmp/fabro/runtime`, v0.336.0): any seed
  about blob/materialization paths should build on this contract.
- SQLite consolidation (blobs 0.335, auth codes 0.336): new "move state
  to SQLite" work should follow this line, not add parallel stores.
- RunIntent / run targets (empty-workspace target in 0.336.0): admission
  plumbing for branch/SHA is moving — seeds touching run targets must
  track it.
- Upstream exit-kind/terminal-status evolution could overlap our
  PublishBlocked/Boundary taxonomy — if upstream ships an equivalent,
  port ours onto it and close the local seed as superseded.
- Graph `on_failure=exit` policy (PR #804, v0.336.0): blocks only the
  unconditional fallback edge for failed nodes; explicit conditions,
  preferred/suggested routes and retry targets still match. Orthogonal to
  our exit kinds (deadlock/soft classify the terminal event; on_failure
  only constrains routing). Do not adopt in `develop` — its edges already
  route every failure explicitly and the exit kinds classify better.

## 2026-08-26 (v0.337.0-nightly.0)

- Model stylesheet templates landed (PR #805): root `model_stylesheet`
  renders via MiniJinja pre-parse with restricted projection
  (`for_model_stylesheet()` = inputs+vars only). Per-node context work
  (fabro-900e) should reuse the `for_*()` restricted-projection pattern
  instead of a new mechanism.
- Watchlist add: `upstream/node-on-failure` branch = node-level
  on_failure override (fabro-types/graph.rs, executor, routing) —
  adjacent to our exit-kind taxonomy; evaluate at next merge.
