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
## 2026-08-26 (v0.337.0-nightly.1)

- node-on-failure LANDED (PR #806) + on_failure="succeed" (PR #811). Reaffirmed:
  do NOT adopt in `develop` — its edges route every failure explicitly, exit kinds
  classify better, and `succeed` would mask quality-gate failures. Orthogonal, no port.
- seed_cycles (fabro-45d0) re-attached behind apply_recorded_outcome_context —
  after_record is now a CONFLICT HOTSPOT whenever upstream reshapes outcome context.
- Watchlist add: upstream PR #751 (open, fork, unreviewed since 2026-08-17) —
  run-level `[run.agent] skill_dirs` (extra dirs, NOT per-node scoping). Does not
  solve fabro-d0d6 (per-stage skill scoping); if it lands, build node-level
  scoping on its LlmSpec/SessionOptions plumbing (model_stylesheet pattern).
  Seed fabro-d0d6 updated with this note (2026-08-26).
- Folder run target (PR #790) gated behind "Local admission": RunIntent admission
  is the validated gate layer — dock future fork admission work (branch/SHA) there.

## 2026-08-27 (post v0.337.0-nightly.1)

- Git run targets gained tag support + PinnedRevision unification (PR #812):
  `RunTarget::Git(GitRunTarget{repo,branch,tag,sha})`; authority sha > tag >
  branch HEAD; no fallback on unavailable tag/commit. Run-target watchlist
  updated: admission matrix is now branch/tag/sha; server preflight still
  passes `clone_tag: None`. Any fork run-target work builds on
  clone_source::PinnedRevision, not a parallel pin enum.
- Title-generation failures now `warn!` with run_id (PR #813) — ops only.
- Stale entries: fabro-890b CLOSED (PR model plumbing landed a1e27c9bf);
  run_workflow.nu no longer exists on meta/denkhaus-lab (lab restructure,
  fabro-a9bb) — verify lab script paths before citing them here.

## 2026-08-31 (v0.339.0-nightly.1, merge 4a22e3d47)

- Daytona now accepts image.docker as snapshot base (DaytonaSnapshotSource
  enum): our validate_daytona_image_settings REMOVED as superseded; the
  both-set rejection lives in upstream's validate_provider_capabilities.
  Our docker-provider mutual exclusion (fabro-969f) kept — no upstream
  equivalent. Seeds 72a0/829a/f251 updated with merge notes; f251 PR offer
  still valid (upstream has no docker autobuild).
- Upstream ships an in-repo product code-review workflow
  (.fabro/workflows/code-review: rules/builtin YAML set, findings/verdict
  schemas, publish_pr.py) and calibrates it on their own PRs — parallel to
  our develop lab dogfooding. Evaluate cross-pollination for the revisor
  line (ADR-0011): structured findings schema + rule_loader are reusable
  concepts.
- SQL run-record foundation landed INACTIVE (RunSummaryStore name kept until
  cutover; migrations 2026082601 automation_run_targets, 2026082701
  run_events, 2026082801 automation_environments; strict legacy run history
  import+verify). Any fork state-to-SQL work must build on this line.
- Automations now REQUIRE server-managed environments and use canonical run
  targets + workflow versions — RunIntent admission stays the validated gate
  layer for fork admission work (branch/tag/SHA matrix from PR #812).
- Chat Completions streams reject tool-call index gaps (fabro-llm) —
  robustness only, no fork impact.

## 2026-09-01 (v0.342.0-nightly.0, merge 2df9dc69a + fix 31809f2ad)

- SQLite run-history CUTOVER completed (was INACTIVE since v0.339): runs +
  run_events now authoritative in SQLite, fail-closed activation with backup +
  legacy import/verify at server start; deletion waits for the writer lock.
  Fork run-store work MUST build on RunSummaryStore/SQLite, not SlateDB paths.
- Upstream bug fixed on our side (fabro-b7c4, 31809f2ad): activation
  rejected real legacy catalog keys (5-segment by-start layout). Parser now
  accepts both; PR offer upstream still pending.
- RunIntent registration spine + local producer support landed: workflow
  versions register dependency-first; `fabro run` resolves local workflow
  packages and observes git targets via remote SHA query
  (remote_branch_sha_noninteractive), not local tracking refs. Run-target
  admission work docks on RunIntent as before.
- RunIntentArgs gained tri-state dry_run/auto_approve/preserve_sandbox wire
  overrides — candidate mechanism for per-run execution overrides without
  config edits (note for develop/revisor design).
- ManifestRepoInfo split origin/push origins (see conflict-policy 2026-09-01):
  our insteadOf canonicalization rides origin_url; push identity compares raw
  config bytes (upstream semantics).
