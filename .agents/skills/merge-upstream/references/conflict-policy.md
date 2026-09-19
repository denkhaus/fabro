# Conflict resolution policy (fabro fork merges)

Core rule: **both sides survive.** Upstream ships the platform direction;
our fork ships feature work that must not regress. Adapt OUR call sites to
upstream's new signatures; never revert upstream, never drop our features.

## Resolution procedure

1. Classify the conflict (see classes below). `git diff --name-only
   --diff-filter=U`, then read each hunk with both sides' context.
2. If both sides ADD tests / helpers / struct fields: keep both, adapt
   names if they collide.
3. If upstream CHANGED a signature we call: take upstream's signature,
   adapt our call sites (find the old-arg mapping in upstream's own
   callers: `git show upstream/main:<file> | grep -n "<fn>("`).
4. If upstream MOVED/RENAMED plumbing (e.g. origin-url derivation): take
   upstream's location, re-attach our additions at the new site.
5. FORBIDDEN SURFACE — upstream edits a file the fork RESTRUCTURED
   (split/renamed/deleted an upstream-owned file; fabro-90ae/PR #242
   class, rejected by user decision 2026-09-18): this state must not
   exist. If you find it, the fork side is the regression — restore the
   upstream file structure (`git show upstream/main:<path>`) and re-home
   any fork additions into fork-owned files per the ab8e pattern. Never
   "resolve" by porting upstream's edits into the fork-shaped modules:
   that cements the divergence and repeats the tax on every future merge.
6. Mechanical marker removal is DANGEROUS on add/add TEST conflicts:
   conflict blocks can swallow closing delimiters (`}`, `);`). After
   resolving, `cargo build` immediately — an "unclosed delimiter" error
   means a test body lost its tail; restore it from
   `git show HEAD:<file>` around the old marker position.

## Conflict classes seen (newest first)

- 2026-08-25 v0.336.0: `demote_large_values_for_prompt` gained
  `budget: usize` param + sandbox-env call shape. Our aggregate-budget
  tests + upstream runtime-directory tests: keep both, adapt our calls
  (git_integration.rs used the old default: pass the literal budget).
- 2026-08-25 v0.336.0: RunSession gained `workflow_path` field — thread
  the existing local `let workflow_path` into the initializer.
- 2026-08-25 v0.336.0: `pr_origin_url` derivation moved to
  `runtime_origin_url` (skip_clone-aware). Take upstream's; keep our
  `pr_content_model` resolution fields alongside.
- 2026-08-25 v0.336.0: `build_single_file_tar` gained `mode: u32`.
  Keep our helper functions; adopt the mode param (Dockerfile: 0o644).
- 2026-08-26 v0.337.0-nightly.1: `after_record` in lifecycle/mod.rs — upstream
  consolidated the manual context sets into `context::apply_recorded_outcome_context`
  (same keys our fork set by hand). Take upstream's helper call, re-attach our
  seed_cycles update (fabro-45d0) behind it. Verify via `-E 'test(seed_cycles)'`.
- 2026-08-26 v0.337.0-nightly.1 (call-site, no conflict marker): `sel.reason` in
  graph/routing.rs is now `fabro_core::graph::EdgeSelectionReason` instead of `&str`;
  string assertions in our local tests fail to COMPILE — adapt to the enum variant.

- 2026-08-27 (post v0.337.0-nightly.1): formatting-identical conflict — both
  sides shipped the same fix (Box::pin(persist_agent_event)); take upstream's
  rustfmt form. Confirm semantic identity first (git hash-object worktree vs
  `git show upstream/main:<file>`), then `git checkout --theirs` is safe ONLY
  when our whole local delta on that file is contained upstream.
- 2026-08-27: upstream struct-variant extraction (RunTarget::Git {..} ->
  Git(GitRunTarget{..})) auto-merges in production code but breaks OUR TEST
  initializers later: E0063 missing field (`clone_tag`) in sandbox_spec.rs
  unit test, E0061 arg count in tests/docker_runner_image.rs — `cargo build`
  stays green; only `cargo test --no-run` catches it. Run a test compile of
  touched packages before declaring conflicts resolved.
- 2026-08-27 (false alarm class): `-p fabro-sandbox` standalone builds warn
  dead_code on push_credentials (docker feature off) and the docker.rs test
  `AsyncWriteExt as _` import looks unused under some feature unions — both
  pre-existing upstream artifacts, NOT merge regressions; clippy --workspace
  is the gate, don't chase them.
- 2026-09-13 v0.355.0: upstream retires the manifest create lane — fork
  tests posting `minimal_manifest_json` bodies to POST /runs get a 422
  `run_intent_invalid`. Adapt to `test_intent_with_bearer` (register version,
  create from id); run settings that the manifest defaults used to supply
  (git author, github permissions) must ride the REGISTERED VERSION's
  workflow.toml (config param), and the create request must carry the
  Authorization bearer or the run records a User subject and capability
  gating changes meaning (bridge tests fabro-e505).
- 2026-09-13 v0.355.0: upstream billing one-rule adds `usage_by_model` to
  CodergenResult::Text/Outcome — fork test backends initializing Text need
  the field; the simulated-agent else branch tuple grows one Vec::new().
- 2026-09-13 v0.355.0: e297 git-source lane is fork-only API surface —
  upstream's create dispatch rewrite must gain our lane back BEFORE the
  RunIntent parse (feature survival), and the yaml requestBody is
  oneOf [RunIntent, GitSourceRunIntent]; progenitor then generates a
  CreateRunBody enum with From impls (typed client wrapper).

## 2026-09-01 (v0.342.0-nightly.0)

- Struct extraction + dual-origin split: upstream reshaped
  `detect_manifest_repo_info` from a tuple into `ManifestRepoInfo { origin_url,
  push_origin_url, branch, sha }` with a BranchPublishStatus publish spine
  (raw-config push identity comparison). Resolution: take upstream's shape
  wholesale, re-attach our insteadOf canonicalization (16bbb8bfb) onto
  `origin_url` ONLY — `push_origin_url` deliberately keeps raw config bytes.
  Splice carefully: the conflict boundary can split the publish doc comment.
- (new failure class, found by DEPLOY not by tests): upstream data-migration
  code can reject REAL production data — v0.342's fail-closed SQLite
  run-history activation died on 'a legacy run-catalog key is not canonical'
  because upstream's parser/tests only know a synthetic key layout, while the
  retired writer emitted `runs/_index/by-start/<YYYY-MM-DD>/<ulid>`
  (RunId::key_segments). Fix locally (accept both layouts, regression-test the
  real writer layout), then offer upstream. A crash-looping container after
  `just up` = read activation logs from `docker logs` + inspect the SQLite
  volume read-only before touching anything; the import is idempotent.

## 2026-09-02 (v0.344.0-nightly.0, merge 3f6681b26)

- Zero-conflict merge with API-REMOVAL fallout (new class): upstream moved
  all run reads to the SQLite read model and deleted
  Database::get_cached_summary / Database::get_cached_run /
  AppState::cached_run. Auto-merge keeps OUR call sites (E0599) plus
  cascading E0308s from unknown scrutinee types (matches! on
  `&run.created_by` binds by value when `run` is an error type — fix the
  E0599 first, the E0308s vanish). Adaptations: ancestry walk ->
  `state.stores.run_summaries.get(&id, Utc::now())`; sandbox inspection ->
  `AppState::cached_run_projection()`; tests -> `Database::
  get_cached_projection()`.
- Test-shape break (E0609): CachedRunProjection now wraps
  `Arc<RunProjection>` (field `summary` -> `projection`), and the SQL
  created_at_ms derives from `run_id.created_at()` via build_summary.
  Don't mutate entries after wrapping — encode later timestamps in the
  RunId itself (`RunId::with_timestamp(later, seq)`).
- Deploy-time: the production preflight query for the session-owner
  unique index can be run READ-ONLY before `just up` (helper container,
  `sqlite3 file:...?mode=ro`); 0 collision groups on 44533 events here.
  Migration log evidence: `fabro_db: Snapshotted SQLite database...` then
  versions 2026082803/2026083101 in `_sqlx_migrations`.

## 2026-08-31 (v0.339.0-nightly.1)

- Import-block widening: our fork widened cfg gates (`any(docker, daytona)`)
  on imports that upstream keeps daytona-only and folds NEW types into
  (DaytonaSnapshotSource + DockerfileSource alias in one use). Resolution:
  keep our wider-gated import, add their new types to the daytona import,
  and dedupe the alias — auto-merge otherwise leaves BOTH import forms.
- Feature-mapping port onto upstream's new code: upstream's new
  DaytonaSnapshotSource match duplicated our Inline/Path mapping inline;
  replace their verbose arms with a call to OUR existing shared helper
  (`sandbox_dockerfile_source`) — both sides' behavior identical, one map.
- Obsolescence reversal in tests: conflict boundary can fall INSIDE our old
  fn head (`fn daytona_image_docker_errors() { let err = ...(`
  vs upstream `fn daytona_image_docker_resolves() { let settings = ...(`).
  Drop our dangling head, take their fn, keep our adjacent feature tests.
- Docs table rows merge PER CELL: upstream semantics for their provider
  column, our cells (docker autobuild) survive in ours; update prose to
  "either provider" when both sides now enforce the same rule.
- Edit mechanics: when splicing conflict blocks by hand, APPLY the cut before
  writing (a forgotten cut glued two fn heads into one line). Build old/new
  from actual file lines + assert marker positions instead of retyping;
  `=======` inside triple-quoted strings invites silent typos.
- (false-alarm class, recurrence): nextest --no-run can warn unused import
  (`tokio_stream::StreamExt as _` in server/tests.rs) that clippy
  --all-targets -D warnings does NOT flag; upstream pre-existing, don't chase.

## 2026-09-03 (v0.345.0-nightly.0, merge 6530c724f)

- Doc-comment stranding (new class): our insertion of `active_run_for_automation`
  had stranded the OLD doc comment of `list_pull_request_creation_candidate_run_ids`
  above our new fn; upstream rewrote that comment + body (064074233). Resolution:
  keep our fn with its own comment, take upstream's new comment, drop the stranded
  lines. Verify the auto-merged body below the markers equals `git show
  upstream/main` before editing (hash the slices).
- Enum-predicate extension (new class): upstream added an exhaustive match over
  an enum OUR fork extended (FailureReason::can_occur_before_start vs our
  Deadlock/SoftStop, fabro-b907). Resolution: add our variants to the new
  predicate with semantics from OUR taxonomy docs (both mid-execution -> false).
- API-removal class, second rename in a row: `AppState::cached_run_projection`
  and `Database::get_cached_projection` -> `load_run_projection` (both levels,
  PR #835 lazy projections; AppState helper owns Option-unwrap + 404). Handler
  call sites and tests adapt.
- (false-alarm recurrence, see 2026-09-02): the tokio_stream unused-import
  warning reappeared on stable nextest --no-run; nightly clippy gate stayed
  clean — confirmed pre-existing upstream, not chased.

- 2026-09-05 (v0.346, first fully conflict-free merge): NEW class 'auto-merge
  orphaned import' — zero textual conflicts, but the merged tests.rs kept OUR
  tokio_stream::StreamExt import that only UPSTREAM's deleted playground tests
  had used; surfaced as unused_imports during the mandatory `nextest --no-run`
  pre-step (would have failed clippy -D warnings later). Resolution: remove the
  import; the one real use in the file was fully qualified (futures_util).
  Lesson: after a conflict-free auto-merge, still grep the 7-ish both-touched
  files for imports whose only users lived in upstream-deleted code.

## 2026-09-06 (v0.348.0-nightly.0, merge of 2f326a13c)

- Squash-divergence class (new): the fork landed v0.347 content via a
  SQUASH merge (PR #30, single-parent commit) — upstream commits were not
  ancestors, so the next `git merge upstream/main` re-counted them. If the
  post-merge tree delta vs HEAD is versions-only, that is why; the merge
  still has value (restores true ancestry + version alignment). Check with
  `git diff HEAD --stat` before resolving.
- Compile-only gate debt (new class): when a previous squash merge gated
  with `cargo check --workspace` only, the NEXT merge's full clippy+nextest
  surfaces pre-existing fork lint/snapshot failures (verify at pre-merge
  HEAD in a temp worktree before calling them regressions). This pass:
  large_futures Box::pin at runner.rs call site (fork-grown future over
  upstream's threshold — pin at the call site, matching the file's local
  pattern), unused_async in fork-only test helper, and two inline snapshots
  missing `approval_timeout_secs` from the fabro-54f0 TTL commit.
- Stale-nextest-binary trap (new, tooling): after edit_file-based snapshot
  fixes, `cargo nextest run` can reuse a stale binary (mtime granularity) —
  the same failure reappears with unchanged line numbers. `touch <file>`
  forces the rebuild; only trust a rerun after it.

## 2026-09-12 (v0.353.0-nightly.0, merge 409a3e9dd — sandbox-driver adoption #849)

- Whole-layer rewrite class: upstream deleted the Sandbox trait AND every
  in-tree provider transport (local.rs/docker.rs-bollard/daytona/push_credentials,
  ~11k lines) for one concrete RunSandbox over git-pinned sandbox-driver
  crates. Resolution is upstream-first per file; fork features re-anchor at
  NEW seams (see touchpoints). Expect E0599/E0308 fallout across every
  consumer, not just conflicted files: Arc<RunSandbox> replaces Arc<dyn Sandbox>,
  local_sandbox()/tool_context() became async, EnvironmentProvider ->
  SandboxProviderKind (LOCAL/DOCKER/DAYTONA consts), labels moved onto
  driver SandboxStatus.
- Trait-decorator features die with the trait: fs_scope's ScopedSandbox
  wrapper ported to the agent TOOL layer (ToolContext checks + FsScope
  filter helpers) instead of resurrecting a sandbox-side wrapper.
- Mock port class: old trait mocks (SlowWrite/Mutable) become driver-handle
  doubles — implement sandbox_driver::Sandbox over MemoryFs/ScriptedExec
  (shared Arc<MemoryFs> for test-side assertions, tokio sleep in write).
  Hand-written local sandbox records in tests MUST use real dirs + the
  Host-provider path-derived id (fabro_sandbox::test_support::local_sandbox_id)
  or attach fails NotFound.
- Regex-edit discipline (recurrence of the 2026-08-31 cut-before-write
  lesson): blanket `fn(...)(?!\.await)` regexes corrupted fn heads and
  nested calls (services_for/tool_context/sandbox_with_file). After each
  batched regex edit, compile immediately; fix paren placement by hand.
- DEPLOY-time data rejection (2nd occurrence of the 2026-09-01 class):
  upstream removed EventBody variants (sandbox.git.*/sandbox.cleanup.*)
  but KEPT the names in is_known_event_name — the strict read path
  crash-looped the server on real stored history. Fix: route
  variantless-but-known names to EventBody::Unknown (fabro-types
  is_legacy_variantless_event_name) + regression test; offer upstream.
  A crash-looping container after `just up` = read activation logs first.
- Smoke count moved: "smoke: all 8 checks green" (was 7) — update any
  hardcoded 7/7 expectations in this skill.

## 2026-09-13 (v0.354.0-nightly.0, merge 6efe0e92c — run-metadata retirement #843 + git identity #856)

- Whole-concept retirement class: upstream deletes a MACHINERY (run-metadata
  branches: RunMetadataRuntime, write_finalize_commit, MetadataSnapshot*)
  our side only has TESTS on. The textual conflicts are tiny (2 hunks), but
  the auto-merge keeps FORK test copies of deleted machinery far from the
  conflict markers (finalize.rs tests referencing write_finalize_commit +
  RunMetadataRuntime + meta_branch at ~1500). After resolving markers, grep
  the WHOLE file (and crate) for the removed type names; expect E0599/E0560
  on fn/fields, and delete those tests with the machinery (upstream-first).
- New-feature fallout on fork tests: #856's RunOptions.git_identity breaks
  fork test initializers with E0063 — add `git_identity: None`. A fork test
  with a FAKE vault GITHUB_TOKEN + declared permissions now hits the real
  GET /user at run start (deterministic failure): isolate with an explicit
  `[run.git.author] name+email` — resolve_git_identity skips the lookup when
  both explicit fields are set.
- Conductor merge leg DISABLED this day (user decision, df7997279 +
  472b9a4a4): merges are local /merge-upstream sessions until fabro-5082 is
  fixed; survey routes Work on drift. Watch: when re-enabling run merges,
  re-check the deploy-proven data-migration tolerance first.

- usage-rename class (v0.356 merge, 2026-09-15): upstream renamed the
  billing vocabulary to usage end to end (#874/#875). Adapt fork emitters to
  the NEW vocabulary while fork_legacy_read keeps reading the OLD one;
  fixtures fail as E0559/E0425 (billing fields, BilledModelUsage) — the
  compiler lists every site, fix mechanically (BilledModelUsage ->
  ModelUsage, billing/billing_by_model -> usage/usage_by_model).

## 2026-09-19 (v0.361.0-nightly.0, merge e5a064b16)

- Schema-rejection fixture class: a hard loader change (codec ->
  codecs) rejects old-shape inline catalogs AT RUNTIME, so fork-added
  test fixtures fail in the suite, not at merge time. Resolution per
  upstream's own migration: the default pair (openai-compatible +
  openai-chat) is simply dropped (http + ["openai-chat"] are the
  defaults); non-default pairs become `codecs = [...]`. Diagnosis
  greps must be UNTRUNCATED (see SKILL.md diagnosis step 3).

## 2026-09-19 (pre-merge seam-shrink class, user directive)

- Diagnosis-first class: BEFORE `git merge`, fork additions found inline
  in upstream-owned files (consts, helpers, doc edits) move into
  fork-owned files on the clean pre-merge tree (behavior-neutral,
  build+tests+clippy+fmt green, committed) — then the merge meets a
  minimal seam instead of a refactored upstream file full of fork text.
  First application: catalog.rs overlay const → fork_catalog.rs (the
  codecs migration then merged against a one-hunk seam).
- Seam mechanics: clippy absolute_paths forbids inline `crate::` paths,
  so the minimal seam is an import line + one call — two small hunks,
  both adjacent-stable. Restore upstream doc comments BYTE-identical
  (diff against `git show <merge-base>:<file>`) — hand-retyped wrapping
  drifts and re-creates a doc hunk.
