Goal: # PR 4 — Extract a source-neutral run-compiler boundary in fabro-server

**Self-contained implementation plan.** Everything needed to implement this
is in this file plus the repository.

**Precondition:** none — this is an independent, behavior-neutral extraction
with no dependency on other in-flight changes. Re-verify the "Verified
current state" section against HEAD before starting; if
`create_run_from_manifest` in
`lib/apps/fabro-server/src/server/handler/runs.rs` or `operations::create` in
`lib/components/fabro-workflow/src/operations/create.rs` has been materially
restructured since the pinned commit, stop and state that in the PR
description instead of adapting blindly.

> **Token notation.** Interpolation tokens are written in this file without
> their enclosing double curly braces, so the file is safe to pass directly
> as a workflow goal (the goal templater would otherwise try to expand them).
> Read `secrets.NAME`, `env.NAME`, `vars.NAME` as the double-curly-brace
> token form used in the codebase, and write the real double-brace syntax in
> the code, tests, and docs you produce.

## Context and goal

When a client POSTs a run to the Fabro server, the server compiles the
request into a persisted, executable run: it normalizes the submitted
workflow bundle, layers settings from server defaults / environment catalog /
workflow config / project config / user config / request args, substitutes
run-scoped variables, parses and validates the Graphviz graph (with template
rendering and model-selector resolution), pins concrete model/provider
choices against the catalog and the set of configured providers, and finally
assembles everything the persistence layer needs to write the run directory
and the `run.created` / `run.submitted` events.

Today that compile pipeline has no single home. It is smeared across three
places:

1. The HTTP handler `create_run_from_manifest`
   (`lib/apps/fabro-server/src/server/handler/runs.rs`) drives the sequence
   inline: prepare, variable snapshot + substitution, run-id resolution,
   sandbox-policy check, provider resolution, input assembly, persistence
   call, plus HTTP error mapping interleaved at every step.
2. `lib/apps/fabro-server/src/run_manifest.rs` owns manifest-to-settings
   preparation (`prepare_manifest_with_environment_defaults`) and the
   persistence-input constructor (`create_run_input`) — which hardcodes
   `submitted_manifest_bytes: None` and `automation: None`, only for the
   handler to overwrite those fields (and `run_id`) after the fact.
3. `operations::create` in
   `lib/components/fabro-workflow/src/operations/create.rs` performs the
   actual graph compilation (parse / transform / validate, with undefined
   template variables promoted to errors) and model pinning
   (`materialize_run`) inside a `spawn_blocking` closure, fused to the
   persistence write in one opaque call.

Why this needs to change: separately planned work will (a) call the compile
pipeline from non-HTTP code paths (server-internal admission/scheduling code
that prepares a run outside a request handler), and (b) feed it workflow
sources other than the current client-submitted manifest (for example a
server-side checkout of a git repository). Both need one typed entry point
whose input speaks in terms of an acquired workflow bundle — not in terms of
the HTTP wire body — and whose stages are individually visible so a future
caller can run acquisition elsewhere or at a different time. None of that
future work lands here; this PR only creates the seam.

**Goal:** after this PR, fabro-server has a single typed "run compiler"
boundary — a module with a source-neutral input type and a typed output —
composed of four internally-separable stages:

1. **Source normalization** — take an already-acquired workflow bundle plus
   an entrypoint path; resolve the entrypoint workflow, its root graph
   source, and bundle-relative references (e.g. dockerfile references in
   config layers resolved against bundled files).
2. **Settings / variables / graph compilation** — layer settings from all
   configured sources, apply the run-variable snapshot, then parse,
   transform, and validate the graph exactly as run-create does today
   (structural render mode, model-resolution transform, undefined template
   variables promoted to hard errors).
3. **Model/provider policy + pinning** — materialize the run against the
   catalog and configured provider set, pinning concrete model and provider
   selections.
4. **Persistence-input assembly** — produce the complete input for the
   persistence layer, with the submitted source bytes, automation reference,
   and resolved run id set once at assembly time instead of patched in
   afterwards.

The manifest-shaped HTTP handler adapts the wire `RunManifest` into the
boundary's input at the edge and keeps all HTTP concerns (status codes,
`ApiError` construction, response shaping) outside the boundary. Behavior is
byte-for-byte unchanged for every endpoint.

Design rules (fixed — do not re-litigate):

- **The boundary's input must not be `RunManifest`** (or any
  `fabro_api::types` request type). The manifest is an accident of today's
  transport; freezing it into the compiler's signature would force every
  future source to fabricate a fake manifest. The manifest-shaped caller
  adapts into the boundary at the edge.
- **No HTTP types inside the boundary.** No `axum` types, `HeaderMap`,
  `StatusCode`, `Response`, or `ApiError` in the boundary module's
  signatures or internals. The boundary returns typed errors; the handler
  maps them to HTTP. A later caller invokes the boundary from non-HTTP
  server code.
- **Byte-for-byte behavior neutrality.** This is a pure extraction. Every
  endpoint's request/response bytes, persisted event contents, error
  messages, log lines, and side-effect ordering must be unchanged. Existing
  fixtures and tests pin behavior; add a pinning test first (see Tests) so
  the refactor is provably neutral.
- **Each stage runs exactly once per create.** Do not build a boundary that
  compiles/validates and then calls a persistence entry point that
  re-compiles internally. If the persistence layer needs restructuring to
  accept already-compiled inputs, restructure it (see step 4) rather than
  running the pipeline twice.
- **The pipeline logic stays in fabro-workflow; the boundary orchestrates
  it.** Do not copy parse/transform/validate/materialize logic into
  fabro-server. Single source of truth: the boundary composes fabro-workflow
  entry points.
- **Fold in the assembly-seam cleanup.** `run_manifest::create_run_input`
  hardcoding `submitted_manifest_bytes: None` / `automation: None` and the
  handler overwriting them (plus `run_id`) post-hoc is a known wart; stage 4
  must accept these as inputs and set them once. No field of the assembled
  persistence input may be mutated after assembly.
- **Async vs blocking is decided per stage by what the stage actually
  touches today** (see the per-stage notes in Implementation step 3), not by
  a blanket choice. CPU-heavy graph compilation stays off the async runtime
  (`spawn_blocking`), as it is today.
- **No new capability.** No intent types, no new wire fields, no OpenAPI
  change, no new workflow-source kinds, no behavior change to any endpoint.
  Separately planned work builds on this seam; this PR only creates it.

## Verified current state (as of origin/main `239490a55`, 2026-07-28 — re-verify before starting)

Line numbers are approximate; the named functions are the stable anchors.

- `lib/apps/fabro-server/src/server/handler/runs.rs`:
  - `create_run` (≈ :519-543) deserializes the body into `RunManifest` and
    delegates to `create_run_from_manifest` with
    `CreateRunFromManifestRequest` (≈ :545-553: manifest, raw submitted
    bytes, optional explicit run id, explicit-title flag, actor, headers,
    optional `AutomationRef`).
  - `create_run_from_manifest` (≈ :555-726) is the whole create pipeline
    inline: `prepare_manifest_with_environment_defaults` (≈ :571-579, errors
    → 400 with the error's message); `snapshot_run_variables` (≈ :580-586,
    errors → 500); `substitute_run_variables` (≈ :587-590, errors → 400
    `Run config variable interpolation failed: ...`); run-id resolution
    `explicit_run_id.or(prepared.run_id).unwrap_or_else(RunId::new)`
    (≈ :591-593); sandbox provider policy check (≈ :594-599, → 400);
    parent-link validation (≈ :600-607); `info!(run_id = %run_id, "Run
    created")` (≈ :608); `resolve_llm_client_with_ready_ids` (≈ :616) with a
    test-support hook `test_run_materialization_provider_ids` behind
    `cfg(any(test, feature = "test-support"))` (≈ :618-630);
    `run_provenance(&headers, &actor)` (≈ :631; fn at ≈ :785);
    `run_manifest::create_run_input(prepared.clone(), ...)` (≈ :632-638)
    followed by the post-hoc mutations `create_input.run_id = Some(run_id)`,
    `create_input.submitted_manifest_bytes = Some(...)`,
    `create_input.automation = automation` (≈ :639-641); then
    `operations::create` (≈ :644-666) with this exact error mapping:
    `ValidationFailed`/`Parse` → 400 `"Validation failed"`,
    `ModelSelection`/`ModelReference` → 400 with the error's display string,
    anything else → 500 `Failed to persist run state: ...`. Post-create side
    effects: cached-summary fetch (≈ :667-680), managed-run map insertion
    (≈ :683-695), spawned title-generation task using `prepared.target_path`
    (≈ :697-719), `201` response (≈ :721-725).
  - The automation paths reuse this same function:
    `server/automation_scheduler.rs` ≈ :264 and `server/handler/
    automations.rs` ≈ :144 call `create_run_from_manifest` directly with
    `automation: Some(..)`. Any signature change to it must keep those
    callers compiling with identical behavior.
  - `run_preflight` (≈ :823-874) and `validate_run_manifest` (≈ :876-920)
    also call `prepare_manifest_with_environment_defaults` +
    `snapshot_run_variables` + `substitute_run_variables`, but then use
    validate-only helpers — they never model-pin the same way create does
    (preflight materializes separately inside `run_manifest::run_preflight`)
    and never persist.
  - `snapshot_run_variables` (≈ :922-926) reads the variable store (async).
    `substitute_run_variables` (≈ :941-953) is pure given the snapshot and
    also validates `run.artifacts.include` globs.
- `lib/apps/fabro-server/src/run_manifest.rs`:
  - `PreparedManifest` (≈ :52-65): cwd, git, root_source, run_id, parent_id,
    title, settings, target_path, workflow_bundle, workflow_input
    (entrypoint `BundledWorkflow`), source_directory.
  - `prepare_manifest_with_environment_defaults` (≈ :79-187): manifest
    version check; `ManifestPath::from_wire` on the target;
    `workflow_bundle_from_manifest` (≈ :301-342) building the
    `WorkflowBundle` from wire keys; entrypoint lookup; args parsing via
    `manifest_args_overrides` (sparse `RunLayer`/`CliLayer`/input
    overrides); `WorkflowSettingsBuilder` layering (server manifest
    defaults + environment defaults + MCP catalog + workflow config layer +
    project config layers + user TOML layers), with dockerfile references in
    config layers resolved against bundled files
    (`settings_layer_with_resolved_dockerfiles`, ≈ :370-386); goal
    extraction; title normalization; run/parent id parsing.
  - `create_run_input` (≈ :236-262) maps `PreparedManifest` →
    `CreateRunInput`, hardcoding `submitted_manifest_bytes: None` and
    `automation: None`. Its only caller is the create handler (≈ runs.rs
    :632).
  - The validate-side helpers (`validate_prepared_manifest*`, ≈ :189-234)
    and preflight/report code in the rest of the file are used by the
    preflight/validate/graph endpoints and by
    `manifest_validation.rs`/`run_tool_manifest.rs` — out of scope here.
- `lib/components/fabro-workflow/src/operations/create.rs`:
  - `CreateRunInput` (≈ :35-59): workflow (`WorkflowInput`), settings, vars,
    cwd, workflow_slug, workflow_path, workflow_bundle,
    submitted_manifest_bytes, run_id, title, automation, git,
    fork_source_ref, parent_id, provenance, configured_providers, web_url.
  - `create` (≈ :87-195): `resolve_workflow` (source.rs; for
    `WorkflowInput::Bundled` it is mostly pure but `resolve_goal_override`
    can read a goal file from disk when `run.goal` is the file variant);
    then a `spawn_blocking` closure (≈ :145-170) running
    `create_from_source` (≈ :288-320) = `preprocess_and_validate`
    (parse/transform/validate with `RenderMode::Structural`,
    `ModelResolutionTransform::for_eligible` + configured default provider,
    ≈ :296-310) + `promote_template_undefined_variables_to_errors`
    (≈ :312-317) + `persist_validated` (≈ :379-426), which calls
    `materialize_run` (≈ :399) for model pinning, builds the `RunSpec`, and
    runs `pipeline::persist` (run-directory writes — blocking I/O). After
    the closure: an optional `workflow.toml` read (≈ :172-175, `None` for
    bundled inputs) and `persist_created_run` (≈ :197-282), which writes
    manifest/definition blobs and appends `run.created` + `run.submitted`.
    Note `persist_created_run` contains a create-or-reopen fallback
    (≈ :209-216) that reopens an existing run store on `create_run` failure
    — a known defect, out of scope (see Scope boundaries).
  - `operations::create`'s only production caller is the server create
    handler (runs.rs ≈ :644); the calls in `operations/start.rs`
    (≈ :1934, :2417) are inside that file's `#[cfg(test)]` module
    (gate at ≈ :1145). `CreateRunInput` is used outside fabro-workflow only
    by `run_manifest.rs`. `operations/mod.rs` (≈ :17) re-exports
    `CreateRunInput`, `CreatedRun`, `create`, `make_run_dir`.
- Existing tests that pin the current pipeline: handler-level create tests
  in `lib/apps/fabro-server/src/server/tests.rs` (≈ :3583, :3627 call
  `create_run_from_manifest` directly); pipeline tests in
  `operations/create.rs`'s test module (e.g.
  `create_persists_normalized_config_and_initial_state`,
  `create_materializes_portable_selectors_for_ready_provider_snapshot_and_pin`,
  `create_returns_validation_failed_with_diagnostics`); manifest-preparation
  tests in `run_manifest.rs`'s test module.

## Implementation

1. **Pin current behavior before touching anything.** In the fabro-server
   test suite (mirror the fixture style of the existing
   `create_run_from_manifest` tests in `server/tests.rs` and the
   `TestAppStateBuilder` helper), add a regression test that drives
   `create_run_from_manifest` with a representative manifest — a bundled
   workflow with a prompt node, an inline goal, args carrying a model
   selector and an input override, a project config layer, a git context,
   and an explicit run id — and asserts the durable outcome precisely: the
   `201` status, and the persisted run's spec/event contents (pinned model
   and provider, rendered graph attributes, settings fields affected by
   layering, labels, provenance, presence of the manifest blob, title).
   Also pin at least one error path per distinct handler mapping: an
   invalid manifest (400 with the preparation error message), an undefined
   `vars.NAME` in a prompt (400 `"Validation failed"`), and an unknown
   model selector (400 with the model-selection error message). Commit this
   test green against the unmodified code; it is the neutrality proof for
   everything below.
2. **Create the boundary module** in fabro-server (suggested:
   `lib/apps/fabro-server/src/run_compiler.rs`, alongside peers like
   `run_manifest.rs`; a directory module is fine if it reads better).
   Define:
   - A source-neutral input type carrying: the acquired `WorkflowBundle` +
     entrypoint `ManifestPath`; settings inputs (server run defaults,
     environment-defaults catalog, MCP server catalog, project config
     sources as path+TOML-source pairs, user config TOML sources,
     args-derived sparse overrides — the `RunLayer`/`CliLayer`/input-override
     shape `manifest_args_overrides` already produces — and the optional
     inline goal override); the run-variable snapshot; identity and lineage
     (resolved run id, parent id, normalized title, git context); the
     configured provider ids; `RunProvenance`; optional web URL; the exact
     submitted source bytes; and the optional `AutomationRef`. Use existing
     fabro-config / fabro-types / fabro-workflow vocabulary for every field;
     no `fabro_api::types` and no axum/HTTP types anywhere in the module.
   - A typed error enum (read `docs/internal/error-handling-strategy.md`
     first) whose variants preserve every distinction the handler's HTTP
     mapping needs: invalid-source/preparation errors, variable
     interpolation errors, validation/parse failures (carrying the
     underlying `fabro_workflow::Error` or equivalent detail),
     model-selection/model-reference errors, and internal errors. The
     handler must be able to reproduce today's status codes and message
     strings exactly from these variants.
   - A typed output: the assembled persistence input (stage 4's product),
     plus whatever compiled artifacts the handler still needs afterwards
     (the entrypoint path for title generation is the known one).
3. **Implement the four stages inside the boundary**, each as its own
   function with typed input/output so they are individually testable and a
   future caller can invoke acquisition separately. Per-stage execution
   model, based on what each touches today:
   - *Stage 1 — source normalization* (pure, synchronous): entrypoint lookup
     in the bundle, root source extraction, and dockerfile-reference
     resolution against bundled files. This subsumes the bundle-facing parts
     of `prepare_manifest_with_environment_defaults`; the manifest-facing
     parts (wire-key parsing, version check, args/config extraction) move to
     the handler-side adapter in step 5.
   - *Stage 2 — settings/variables/graph compilation*: settings layering via
     `WorkflowSettingsBuilder` and variable substitution (reuse the logic of
     `substitute_run_variables`, including its artifact-glob validation) are
     pure given the snapshot — the snapshot itself is an input, taken by the
     caller. Graph compilation must keep running through fabro-workflow's
     pipeline (`resolve_workflow` + `preprocess_and_validate` with
     `RenderMode::Structural` and the eligible-provider model-resolution
     transform, then promoting undefined template variables to errors) and
     must stay on `spawn_blocking` — it is CPU-heavy and can touch the
     filesystem (goal-file override). Whether stages 2-4 share one blocking
     closure (as today) or are separately dispatched is the implementer's
     call; the criterion is that blocking work never runs directly on the
     async runtime and the observable behavior is unchanged.
   - *Stage 3 — model/provider policy + pinning*: `materialize_run` with the
     catalog and configured providers — pure CPU; keep it adjacent to stage
     2's blocking context as it is today.
   - *Stage 4 — persistence-input assembly* (pure): build the complete
     persistence input with run id, submitted source bytes, and automation
     reference populated from the boundary input. Delete the
     assemble-then-mutate pattern entirely.
4. **Open a persist-without-recompile seam in fabro-workflow.** Today
   `operations::create` fuses compile and persist, so a boundary that
   compiles would trigger a second compile when calling it. Restructure
   `operations/create.rs` so the compile portion (resolve +
   preprocess/validate + promote + materialize) and the persist portion
   (`RunSpec` assembly + `pipeline::persist` + `persist_created_run`) are
   separately callable, then reimplement `create` as their composition so
   its existing signature and behavior are preserved for current users
   (including its own test module). The server boundary calls the compile
   pieces from its stages 2-3 and the persist piece with stage 4's output.
   Mirror the file's existing internal split (`create_from_source` /
   `persist_validated` / `persist_created_run`) rather than inventing a new
   pipeline shape; the work is mostly making the seams `pub` (or
   `pub(crate)`-plus-re-export) with honest input structs, not rewriting
   logic. Do not duplicate any of this logic into fabro-server.
5. **Rewire `create_run_from_manifest` as edge adapter + boundary caller.**
   The handler keeps its signature (its automation callers must not change)
   and becomes: deserialize/validate the manifest shape and convert to the
   boundary input (manifest version check, wire-key parsing via
   `workflow_bundle_from_manifest`, `manifest_args_overrides`, config
   extraction by type, goal/title/run-id/parent-id extraction — reusing the
   existing `run_manifest.rs` functions where they are already
   manifest-shaped); take the variable snapshot; resolve the run id and
   compute provenance from headers at the edge; run the same pre-checks in
   the same order (sandbox provider policy, parent-link validation) with
   identical status codes and messages; call the boundary; map its typed
   errors to today's exact HTTP responses; then perform the unchanged
   post-create side effects (summary fetch, managed-run insertion, title
   generation task, `201`). Keep the `info!(run_id = %run_id, "Run
   created")` log at the equivalent point and keep the test-support
   provider-ids hook at the edge with the same `cfg` gating. Delete
   `run_manifest::create_run_input` once nothing calls it.
6. **Doc comments on the boundary.** State what the boundary is (the single
   create-time compile pipeline), what each stage consumes and produces, why
   the input is source-neutral, and that callers own source acquisition,
   variable snapshotting, and (for HTTP callers) all wire mapping.

## Scope boundaries — deliberately NOT in this PR

- **New request types, workflow-source kinds, or wire/OpenAPI changes** —
  none. Do not touch `docs/public/api-reference/`. The create endpoint keeps
  accepting exactly today's manifest body; a future request shape is known
  follow-up work that will adapt into this boundary the same way the
  manifest does.
- **The preflight, validate, and graph endpoints, `manifest_validation.rs`,
  and `run_tool_manifest.rs`** — leave them on
  `prepare_manifest_with_environment_defaults` and the validate helpers
  as-is, even where that leaves some duplication with the new boundary.
  Migrating those surfaces is known follow-up work; forcing them through the
  compiler now would change their behavior (they deliberately do not pin
  models or persist).
- **When/where compile runs** — the boundary is called at create time from
  the create handler, exactly as today. Do not move compilation into
  admission/scheduling code paths; that is separately planned work this seam
  exists to enable.
- **fabro-store** — untouched. No changes to event schemas, append
  semantics, or blob storage.
- **The create-or-reopen fallback in `persist_created_run`**
  (operations/create.rs ≈ :209-216, reopening an existing run store and
  appending another `run.created`) — leave as-is, including when moving code
  around it. It is a known defect with separately planned work; "fixing" it
  here would be a behavior change in a PR that promises none.
- **The automation scheduler and automation materializer** — leave their
  call paths as-is; they funnel through `create_run_from_manifest` and get
  the boundary for free.
- **Handler side-effect behavior** — title generation, managed-run map
  bookkeeping, summary decoration, and response shaping stay exactly as they
  are; they are the handler's job, not the compiler's.
- **`RunSpec`, `run.created` event contents, and `run_manifest.rs`'s
  preflight/report code** — no field additions, removals, or renames.

If work outside these boundaries seems genuinely required for this PR to
compile or pass its tests, stop and state that in the PR description rather
than expanding scope.

## Tests

This is a pure extraction, so the emphasis is pin-first rather than
failing-first: the step-1 regression test is written and committed against
the unmodified code, then must stay green untouched through the refactor.
All tests hermetic — temp-dir fixtures, in-memory stores, no ambient
provider keys (use the existing test catalogs and `TestAppStateBuilder`
patterns).

1. **Handler-output pinning test** (step 1) — the representative manifest
   produces identical persisted spec/event contents and HTTP responses
   before and after the extraction, including the three pinned error paths.
   *Property: the extraction is behavior-neutral at the wire and in the
   event log.*
2. **Boundary unit tests per stage**, in the new module:
   - stage 1: entrypoint resolution and a dockerfile reference resolved
     against bundle files; a missing entrypoint and a missing bundled
     dockerfile produce the same error messages as today.
   - stage 2: settings layering precedence (server default overridden by
     project layer overridden by args override), variable substitution
     (a `vars.NAME` reference in run settings resolves from the snapshot;
     an artifact-include glob error surfaces), and graph compilation
     (undefined `vars.NAME` in a prompt is a hard error; a defined one
     renders — mirror the existing
     `vars_resolve_in_node_prompt_through_create_pipeline` /
     `unknown_var_in_prompt_warns_at_validate_then_errors_at_run_create`
     coverage in operations/create.rs).
   - stage 3: a portable model selector pins to the expected
     model/provider for a given configured-provider set (mirror
     `create_materializes_portable_selectors_for_ready_provider_snapshot_and_pin`
     with the small portable test catalog).
   - stage 4: the assembled persistence input carries the submitted source
     bytes, automation reference, and resolved run id exactly as provided —
     pinning that the post-hoc-mutation seam is gone.
3. **fabro-workflow seam test** — `operations::create` reimplemented as
   compile+persist composition still passes its entire existing test module
   unchanged, and the new persist-precompiled entry point produces the same
   `CreatedRun`/durable state as `create` for the same input.
4. **Full workspace suite** — the reducer, lifecycle, handler, automation,
   and CLI test suites are the regression net; run
   `cargo nextest run --workspace` and treat any diff as a neutrality
   violation to fix, not a snapshot to accept. If an insta snapshot changes,
   the refactor broke neutrality — do not run a blanket
   `cargo insta accept`.

## Acceptance / verification

- `cargo +nightly-2026-04-14 fmt --check --all`
- `cargo +nightly-2026-04-14 clippy --workspace --all-targets -- -D warnings`
- `cargo nextest run --workspace`
- No OpenAPI/wire change (do not touch `docs/public/api-reference/`).
- `cargo build --workspace` without the `test-support` feature still
  succeeds if any test helper was added behind it.
- `run_manifest::create_run_input` no longer exists; no call site mutates a
  persistence input after assembly.
- The new boundary module has no dependency on `axum`, `fabro_api::types`
  request types, or anything HTTP-shaped (verify by reading its imports).

## Conventions

- Read `docs/internal/error-handling-strategy.md` before adding the
  boundary's error type, and `docs/internal/logging-strategy.md` before
  moving or adding any `tracing` call sites; keep existing log lines' fields
  and levels unchanged.
- Never print or log a resolved secret value, including from tests.
- Plain-English commit messages, PR text, and comments — describe what the
  change does; no internal planning identifiers or plan-file names in
  anything that ships.
- PR description must state plainly: (1) this is a pure refactor with no
  behavior change — every endpoint's requests, responses, persisted events,
  and error messages are byte-for-byte unchanged, and a pinned regression
  test written before the refactor proves it; (2) what the new boundary is —
  a single typed, source-neutral entry point in fabro-server for the
  create-time compile pipeline (source normalization, settings/variables/
  graph compilation, model pinning, persistence-input assembly); (3) why it
  exists — so the compile pipeline has one home that future non-HTTP server
  code paths and alternative workflow sources can call, instead of logic
  smeared across the HTTP handler, the manifest-preparation module, and the
  workflow-operations internals.
- If implementation uncovers a hidden behavioral coupling that makes a stage
  impossible to extract without changing observable behavior, stop and
  surface it in the PR description rather than working around it.


## Completed stages
- **toolchain**: succeeded
  - Script: `command -v cargo >/dev/null || { curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y && sudo ln -sf $HOME/.cargo/bin/* /usr/local/bin/; }; cargo --version 2>&1`
  - Output:
    ```
    cargo 1.96.0 (30a34c682 2026-05-25)
    ```
- **preflight_compile**: succeeded
  - Script: `cargo check -q --workspace 2>&1`
  - Output: (empty)
- **preflight_lint**: succeeded
  - Script: `cargo +nightly-2026-04-14 clippy -q --workspace --all-targets -- -D warnings 2>&1`
  - Output: (empty)


Read the plan file referenced in the goal and implement every step. Make all the code changes described in the plan. Use red/green TDD.