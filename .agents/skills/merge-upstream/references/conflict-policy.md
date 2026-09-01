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
5. Mechanical marker removal is DANGEROUS on add/add TEST conflicts:
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
