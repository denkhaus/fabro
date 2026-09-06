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

## 2026-09-02 (v0.344.0-nightly.0, merge 3f6681b26)

- SQLite read-model consolidation COMPLETE: run queries, PR recovery, and
  session ownership all read SQLite now (PR #830/#829); SlateDB shrinks to
  event log + warm projection cache. Any fork read path builds on
  RunSummaryStore::get / Database::get_cached_projection.
- Session ownership: unique partial index on run_events(session_id) WHERE
  event_name='run.session.created', fail-closed legacy preflight, snapshot
  before new migrations. sessions handlers (attach/adopt) ride
  find_session_owner — fabro-204e (attach replay UX) unchanged, still open.
- Automations: independent git workflow sources (repo+branch+optional
  tag/sha aligned with run targets, PR #825); materializer exposes a
  GitRemote resolver seam (git_checkout.rs resolve-then-prepare, credential
  reuse) — reuse it for any fork feature needing a credentialed checkout
  (revisor line) instead of new clone plumbing.
- fabro-b7c4: upstream STILL lacks the 5-segment by-start catalog fix;
  local fix 31809f2ad survived the merge untouched; PR offer still valid.
- Push race (environmental): a background watcher pushes origin/denkhaus
  within ~30s of any local commit — `git merge upstream/main` auto-commits
  and can get pushed BEFORE the proper merge message/adaptations land.
  Merge with `--no-commit`, finish fixes + message, then commit and push
  once. If raced anyway: `--force-with-lease` over the default-message
  merge is safe when nothing built on it (check origin/denkhaus parents).

## 2026-09-03 (v0.345.0-nightly.0, merge 6530c724f)

- Run-read touchpoint RENAMED again: any fork read path builds on
  `AppState::load_run_projection` / `Database::load_run_projection` (on-demand:
  active snapshot OR SQLite replay; PR #835). `cached_run_projection` and
  `get_cached_projection` no longer exist.
- CLI runs are now intent-created from immutable workflow versions (PR #831):
  project/CLI-machine `[run]`+`[environments]` settings are NO LONGER
  transmitted by `fabro run`/`fabro create`. Workflow behavior (incl.
  `[run.pull_request]`, environment pinning) must live in workflow.toml or
  server-managed environments. Watch: manual `fabro run` of workflows relying
  on `.fabro/project.toml` defaults (implement-issue uses project toolchain env).
- fabro-b440 crash-loop half FIXED upstream (PR #838 pre-start failure
  persistence); seed updated with merge note — resume-mode UX still open.
- fabro-696c: upstream PR #837 now rejects auto-PRs for Local environments at
  admission (`pull_request_environment_unsupported`) — complements, not
  supersedes (Docker runs still need PR-capable credentials).
- Automations: scheduler run creation rides `create_run_from_intent`, so the
  Local+auto-PR rejection applies to automations uniformly; our Docker
  environments unaffected.

- 2026-09-05 (v0.346): RunStatusKind::Runnable joined
  reconcile_incomplete_runs_on_startup (PR #841) — server restarts now emit
  RunFailed for admitted-but-unstarted runs; our Slack lifecycle routes
  receive those correctly. Interacts with future revisor cron (ADR-0013
  phase 3): restarts fail scheduled runs cleanly instead of zombie-ing.
- 2026-09-05 (v0.346): playground fully removed (PR #839) — if any future
  seed or doc references /playground or POST /api/v1/playground/chat, treat
  as dead; PR #533 superseded upstream.
- 2026-09-05 (v0.346): Client::list_environments() is now available for
  engine-side use — candidate infra for fabro-8d30 part a (availability
  probe UX), watch when that seed is picked up.

## 2026-09-06 (v0.348.0-nightly.0, merge 2f326a13c)

- All touchpoints re-verified green post-merge (see
  `.fabro/reports/merge-upstream/2f326a13c.md`); none superseded.
- fabro-54f0 (approval TTL backstop) had landed WITHOUT updating the attach
  and inspect inline snapshots (`approval_timeout_secs` 86400/null) — fixed
  during this merge's gate. Future features that add run-spec settings
  fields must sweep `fabro-cli` it-test snapshots in the same commit.
- lab-check.yml is RETIRED (lab restructure): the auto-merge wiring
  touchpoint now resolves to `.github/workflows/dogfood-gate.yml` + branch
  protection (engine-owned).
