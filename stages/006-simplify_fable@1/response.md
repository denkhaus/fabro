Review complete. Three parallel review agents (reuse, quality, efficiency) analyzed the shared-checkout parallel execution diff, and I fixed the verified findings.

## Fixed

**Correctness (high)**
- **Old stored events failed to deserialize**: `ParallelBranchResult.context_updates` was a required serde field, so pre-change `parallel.completed` events (which lacked it) would hard-fail event replay. Added `#[serde(default)]`.

**Stringly-typed status → `StageOutcome` (medium)**
- `ParallelBranchResult.status`, `ParallelBranchCompletedProps.status`, the workflow `Event` variant, and the CLI `ProgressEvent` all carried `String` where the existing `StageOutcome` enum (same serde strings, existing OpenAPI schema + build.rs replacement) applied. Typed them end-to-end: OpenAPI now `$ref`s `StageOutcome`, regenerated the TS client, and `run_state.rs` dropped its parse-back `from_str`. This also eliminated the mixed `result.status == "failed"` / `failure_category()` predicate in the reap loop.

**Payload lifecycle efficiency (high/medium)**
- **Offload before emit**: `parallel.completed` previously carried raw (un-offloaded) branch results into the event log, projection, and per-event projection-cache clones. The handler now runs the new `artifact::offload_parallel_branch_updates` on the typed results before emitting/returning them.
- **Removed the double-apply**: dropped the handler's direct `context.apply_updates(...)` (updates flow only through `outcome.context_updates`, like every other handler), which also kept raw results out of `StageCompleted.context_values`.
- **Reduced per-branch copies**: parent snapshot taken once and shared via `Arc` (was 3 full context clones per branch), `graph` cloned once into an `Arc`, results moved instead of cloned into the event, removed the no-op `sort_by_key`, and `materialize_blob_ref` now checks file existence before reading the blob (content-addressed, so safe). Added a cheap size fast-path to `offload_value` for short strings.

**Code reuse / structure**
- `EngineServices` now derives `Clone` — deleted the field-by-field reconstruction in the branch task.
- Extracted shared `context_diff` into `context.rs` (was duplicated in `manager_loop` and reimplemented in `parallel.rs`), taking `after` by value to move entries.
- Added `WorkflowContext::parsed_run_id()` replacing identical 4-line parse blocks in `parallel.rs` and `agent.rs`.
- Replaced the 5-tuple `handles` collection with a named `BranchDispatch` struct; unified simulated/normal notes with a prefix variable.
- Unified `FanInHandler::simulate`/`execute` copy-paste twins into `run_join(..., simulated)`; fan-in now validates result shape via a slim deserialize instead of cloning the full hydrated payload it never used.
- Fixed the stale `stage_scope.rs` doc referencing removed worktree/`GitCommit` behavior and renamed the change-narrating `..._omits_git_state_...` test.

**Web**
- Deleted dead `extractStageNotes` and `asUnknownRecord` (production callers removed by this change) plus their tests.
- `parallel-children.tsx` now reuses shared `stageStatusTone`/`stageStatusLabel` from `stage-sidebar` (dropping its parallel tone/label maps with the impossible `cancelled` key) via a local `BranchRow` view type; `parseParallelOverview` validates statuses against the typed `StageOutcome` enum.
- New tests use the existing `setupReactTestEnv` (with teardown) and a new shared `makeEventEnvelope` fixture builder in `test-utils`.

## Validated
`cargo build -p fabro-api` + API round-trip tests, TS client regen + typecheck, `cargo nextest run --workspace` (7,007 passed), `fmt --check`, `clippy -D warnings`, web `bun test` (679 passed) + typecheck + build, and `cargo dev docs check` — all clean.

Noted but intentionally skipped: per-run blob memoization across resolution passes and concurrent blob resolution (worthwhile but a larger cross-cutting change beyond this cleanup's scope), and the negative string-absence assertions in new tests (harmless short-lived regression guards).