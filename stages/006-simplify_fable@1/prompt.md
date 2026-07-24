Goal: <proposed_plan>
# Shared-Checkout Parallel Execution

## Summary

Simplify parallel execution so every branch:

- Receives an isolated context fork.
- Executes concurrently against the same sandbox and Git checkout.
- Creates no Git branch, worktree, checkpoint, or branch commit.
- Always waits for every branch; remove `first_success` and `join_policy`.
- Produces a collision-free result containing its context updates.

Fan-in remains an explicit join node. Without a prompt it is a no-op barrier; with a prompt it behaves like a standard prompt stage that reduces the aggregated results without selecting a branch.

## Implementation Changes

### Parallel execution

- Remove all parallel-specific Git setup, checkpointing, committing, cleanup, and fast-forwarding.
- Give each branch the same `Arc<dyn Sandbox>` and inherited `internal.work_dir`.
- Retain `max_parallel`, branch-scoped contexts, IDs, hooks, and lifecycle events.
- Preserve outgoing-edge order in the final results regardless of completion order.
- Define the parallel outcome as:
  - `succeeded` when every branch succeeds.
  - `failed` when every branch fails.
  - `partially_succeeded` for every mixed or partial result.
- Preserve branch ID/index when handler or task execution fails.
- Keep the current single-node-per-branch topology.

### Results and fan-in

Introduce a shared typed result:

```rust
ParallelBranchResult {
    id: String,
    status: String,
    context_updates: BTreeMap<String, serde_json::Value>,
}
```

- Store the ordered array in `parallel.results` and keep `parallel.branch_count`.
- Include updates from successful and failed branches; never merge them into the parent context at top level.
- Offload large leaf values using existing artifact storage while keeping `parallel.results` itself a structured array.
- Resolve nested textual `response.*` and `command.output` blob references when building downstream prompt context.
- Emit the same typed results through `parallel.completed` and project them into `StageProjection.parallel_results`.
- A promptless fan-in verifies results exist and succeeds with a joined-branches note.
- A prompted fan-in uses standard prompt execution and records `response.<fan_in_id>`, `last_response`, usage, and prompt events. It performs no ranking or selection.

Remove:

- Heuristic and LLM branch-selection code.
- `parallel.fan_in.best_id`, `best_outcome`, and `best_head_sha`.
- Per-result `head_sha` and `score`.
- The special fan-in model-usage mode.
- Fan-in selected-branch notes and UI.

### Delete obsolete Git/worktree infrastructure

- Delete the parallel-only `WorktreeSandbox`, its options/callbacks, exports, tests, and sandbox trait path helpers.
- Remove `EngineServices`’ parallel-only `GitState` and pipeline wiring.
- Remove now-unused branch/worktree/fast-forward Git helpers and parallel ref sanitization.
- Remove the parallel-base-checkpoint notice code.
- Remove the now-unemitted `git.branch`, `git.worktree.added`, and `git.worktree.removed` event variants and property types.
- Leave the server’s independent Git checkout/worktree implementation unchanged.
- Continue normal run-level checkpointing after the parallel node; any shared workspace changes are captured together.

## Public Interfaces and Documentation

- Remove `join_policy` from DOT documentation, examples, fixtures, events, and UI.
- Add a validation error directing users to remove any `join_policy` attribute.
- Update `parallel.started` to contain only visit and branch count.
- Update `parallel.branch.completed` to remove `head_sha`.
- Define `ParallelBranchResult` in OpenAPI and change `StageProjection.parallel_results` from untyped objects to that typed array; regenerate Rust and TypeScript clients.
- Update the parallel UI to show branch status and links without commit SHAs or a join-policy badge.
- Replace the fan-in trophy/selection UI with joined-state information and the optional standard reducer transcript.
- Rewrite the active parallel strategy and public docs around shared-checkout concurrency:
  - Read-only behavior is best effort.
  - Concurrent writes are allowed but entirely user-managed.
  - Fabro performs no write enforcement, detection, or warnings.
  - Results are available through `parallel.results`, not a workspace `parallel_results.json`.
  - A fan-in prompt synthesizes results but never selects workspace state.
- Remove `join_policy` from all checked-in tutorial/demo workflows and generated documentation fixtures.

## Test Plan

- Unit-test that every branch receives the same sandbox working directory while retaining independent contexts.
- Verify deterministic result ordering and complete per-branch context updates, including failed branches and structured/command outputs.
- Cover all-success, mixed, partial, all-failed, zero-branch, `max_parallel`, dry-run, and run-cancellation behavior.
- In a temporary Git repository, run parallel branches that write distinct files and assert:
  - Both files remain in the shared checkout.
  - No `fabro/run/parallel/*` refs exist.
  - No parallel worktrees, branch commits, worktree events, or fast-forward commands occur.
- Test promptless fan-in as a no-op join and prompted fan-in as a standard reducer that sees every branch result and emits a normal response.
- Update event serialization, store projection, API round-trip, web parser, and renderer tests for the new typed payloads and removed fields.
- Remove the obsolete host and Daytona parallel-Git-selection tests; retain provider-independent shared-sandbox coverage.
- Verify with:
  - `cargo build -p fabro-api`
  - `cd lib/packages/fabro-api-client && bun run generate && bun run typecheck`
  - `cargo build --workspace`
  - `ulimit -n 4096 && cargo nextest run --workspace`
  - `cargo +nightly-2026-04-14 fmt --check --all`
  - `cargo +nightly-2026-04-14 clippy --workspace --all-targets -- -D warnings`
  - `cd apps/fabro-web && bun test && bun run typecheck && bun run build`
  - `cargo --locked dev docs check`

## Assumptions and Unresolved Questions

- The application is greenfield; removed fields, events, and attributes receive no compatibility layer.
- Parallel branches remain single-node executions.
- `max_parallel` remains supported.
- Shared-workspace write races are explicitly the user’s responsibility.
- No Git workspace warning or read-only enforcement will be added.
- Unresolved questions: none.
</proposed_plan>


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
- **implement**: succeeded
  - Model: openai/gpt-5.6-sol, 45.3k tokens in / 135.5k out
  - Files: /home/daytona/workspace/fabro/apps/fabro-web/app/components/stage-renderers/fan-in-results.test.tsx, /home/daytona/workspace/fabro/apps/fabro-web/app/components/stage-renderers/fan-in-results.tsx, /home/daytona/workspace/fabro/apps/fabro-web/app/components/stage-renderers/helpers.test.ts, /home/daytona/workspace/fabro/apps/fabro-web/app/components/stage-renderers/helpers.ts, /home/daytona/workspace/fabro/apps/fabro-web/app/components/stage-renderers/parallel-children.test.tsx, /home/daytona/workspace/fabro/apps/fabro-web/app/components/stage-renderers/parallel-children.tsx, /home/daytona/workspace/fabro/apps/fabro-web/app/routes/run-stages.tsx, /home/daytona/workspace/fabro/docs/internal/events.md, /home/daytona/workspace/fabro/docs/internal/parallel-strategy.md, /home/daytona/workspace/fabro/docs/public/api-reference/fabro-api.yaml, /home/daytona/workspace/fabro/docs/public/examples/clone-substack.mdx, /home/daytona/workspace/fabro/docs/public/execution/context.mdx, /home/daytona/workspace/fabro/docs/public/execution/outcomes.mdx, /home/daytona/workspace/fabro/docs/public/reference/dot-language.mdx, /home/daytona/workspace/fabro/docs/public/tutorials/ensemble.mdx, /home/daytona/workspace/fabro/docs/public/tutorials/parallel-review.mdx, /home/daytona/workspace/fabro/docs/public/workflows/stages-and-nodes.mdx, /home/daytona/workspace/fabro/lib/crates/fabro-agent/src/lib.rs, /home/daytona/workspace/fabro/lib/crates/fabro-agent/src/sandbox.rs, /home/daytona/workspace/fabro/lib/crates/fabro-api/build.rs, /home/daytona/workspace/fabro/lib/crates/fabro-api/src/lib.rs, /home/daytona/workspace/fabro/lib/crates/fabro-api/tests/run_event_round_trip.rs, /home/daytona/workspace/fabro/lib/crates/fabro-api/tests/stage_projection_round_trip.rs, /home/daytona/workspace/fabro/lib/crates/fabro-cli/src/commands/run/run_progress/mod.rs, /home/daytona/workspace/fabro/lib/crates/fabro-cli/tests/it/workflow/dry_run_examples.rs, /home/daytona/workspace/fabro/lib/crates/fabro-dump/src/lib.rs, /home/daytona/workspace/fabro/lib/crates/fabro-sandbox/src/daytona/mod.rs, /home/daytona/workspace/fabro/lib/crates/fabro-sandbox/src/docker.rs, /home/daytona/workspace/fabro/lib/crates/fabro-sandbox/src/lib.rs, /home/daytona/workspace/fabro/lib/crates/fabro-sandbox/src/sandbox.rs, /home/daytona/workspace/fabro/lib/crates/fabro-store/src/run_state.rs, /home/daytona/workspace/fabro/lib/crates/fabro-store/tests/serializable_projection.rs, /home/daytona/workspace/fabro/lib/crates/fabro-types/src/lib.rs, /home/daytona/workspace/fabro/lib/crates/fabro-types/src/parallel.rs, /home/daytona/workspace/fabro/lib/crates/fabro-types/src/run_event/infra.rs, /home/daytona/workspace/fabro/lib/crates/fabro-types/src/run_event/misc.rs, /home/daytona/workspace/fabro/lib/crates/fabro-types/src/run_event/mod.rs, /home/daytona/workspace/fabro/lib/crates/fabro-types/src/run_projection.rs, /home/daytona/workspace/fabro/lib/crates/fabro-validate/src/lib.rs, /home/daytona/workspace/fabro/lib/crates/fabro-validate/src/rules/inert_attribute.rs, /home/daytona/workspace/fabro/lib/crates/fabro-validate/src/rules/join_policy_removed.rs, /home/daytona/workspace/fabro/lib/crates/fabro-validate/src/rules/mod.rs, /home/daytona/workspace/fabro/lib/crates/fabro-workflow/README.md, /home/daytona/workspace/fabro/lib/crates/fabro-workflow/src/artifact.rs, /home/daytona/workspace/fabro/lib/crates/fabro-workflow/src/context.rs, /home/daytona/workspace/fabro/lib/crates/fabro-workflow/src/event/convert.rs, /home/daytona/workspace/fabro/lib/crates/fabro-workflow/src/event/emitter.rs, /home/daytona/workspace/fabro/lib/crates/fabro-workflow/src/event/events.rs, /home/daytona/workspace/fabro/lib/crates/fabro-workflow/src/event/names.rs, /home/daytona/workspace/fabro/lib/crates/fabro-workflow/src/git.rs, /home/daytona/workspace/fabro/lib/crates/fabro-workflow/src/handler/fan_in.rs, /home/daytona/workspace/fabro/lib/crates/fabro-workflow/src/handler/manager_loop.rs, /home/daytona/workspace/fabro/lib/crates/fabro-workflow/src/handler/parallel.rs, /home/daytona/workspace/fabro/lib/crates/fabro-workflow/src/pipeline/execute.rs, /home/daytona/workspace/fabro/lib/crates/fabro-workflow/src/pipeline/initialize.rs, /home/daytona/workspace/fabro/lib/crates/fabro-workflow/src/sandbox_git.rs, /home/daytona/workspace/fabro/lib/crates/fabro-workflow/src/services.rs, /home/daytona/workspace/fabro/lib/crates/fabro-workflow/src/test_support.rs, /home/daytona/workspace/fabro/lib/crates/fabro-workflow/tests/it/daytona_integration.rs, /home/daytona/workspace/fabro/lib/crates/fabro-workflow/tests/it/git_integration.rs, /home/daytona/workspace/fabro/lib/crates/fabro-workflow/tests/it/integration.rs, /home/daytona/workspace/fabro/test/attractor/reference_template.dot, /home/daytona/workspace/fabro/test/docs/examples/clone-substack/clone-substack.fabro


# Simplify: Code Review and Cleanup

Review all changed files for reuse, quality, and efficiency. Fix any issues found.

## Phase 1: Identify Changes

Run \`git diff\` (or \`git diff HEAD\` if there are staged changes) to see what changed. If there are no git changes, review the most recently modified files that the user mentioned or that you edited earlier in this conversation.

## Phase 2: Launch Three Review Agents in Parallel

Use the ${AGENT_TOOL_NAME} tool to launch all three agents concurrently in a single message. Pass each agent the full diff so it has the complete context.

### Agent 1: Code Reuse Review

For each change:

1. **Search for existing utilities and helpers** that could replace newly written code. Look for similar patterns elsewhere in the codebase — common locations are utility directories, shared modules, and files adjacent to the changed ones.
2. **Flag any new function that duplicates existing functionality.** Suggest the existing function to use instead.
3. **Flag any inline logic that could use an existing utility** — hand-rolled string manipulation, manual path handling, custom environment checks, ad-hoc type guards, and similar patterns are common candidates.

### Agent 2: Code Quality Review

Review the same changes for hacky patterns:

1. **Redundant state**: state that duplicates existing state, cached values that could be derived, observers/effects that could be direct calls
2. **Parameter sprawl**: adding new parameters to a function instead of generalizing or restructuring existing ones
3. **Copy-paste with slight variation**: near-duplicate code blocks that should be unified with a shared abstraction
4. **Leaky abstractions**: exposing internal details that should be encapsulated, or breaking existing abstraction boundaries
5. **Stringly-typed code**: using raw strings where constants, enums (string unions), or branded types already exist in the codebase
6. **Unnecessary JSX nesting**: wrapper Boxes/elements that add no layout value — check if inner component props (flexShrink, alignItems, etc.) already provide the needed behavior
7. **Unnecessary comments**: comments explaining WHAT the code does (well-named identifiers already do that), narrating the change, or referencing the task/caller — delete; keep only non-obvious WHY (hidden constraints, subtle invariants, workarounds)

### Agent 3: Efficiency Review

Review the same changes for efficiency:

1. **Unnecessary work**: redundant computations, repeated file reads, duplicate network/API calls, N+1 patterns
2. **Missed concurrency**: independent operations run sequentially when they could run in parallel
3. **Hot-path bloat**: new blocking work added to startup or per-request/per-render hot paths
4. **Recurring no-op updates**: state/store updates inside polling loops, intervals, or event handlers that fire unconditionally — add a change-detection guard so downstream consumers aren't notified when nothing changed. Also: if a wrapper function takes an updater/reducer callback, verify it honors same-reference returns (or whatever the "no change" signal is) — otherwise callers' early-return no-ops are silently defeated
5. **Unnecessary existence checks**: pre-checking file/resource existence before operating (TOCTOU anti-pattern) — operate directly and handle the error
6. **Memory**: unbounded data structures, missing cleanup, event listener leaks
7. **Overly broad operations**: reading entire files when only a portion is needed, loading all items when filtering for one

## Phase 3: Fix Issues

Wait for all three agents to complete. Aggregate their findings and fix each issue directly. If a finding is a false positive or not worth addressing, note it and move on — do not argue with the finding, just skip it.

When done, briefly summarize what was fixed (or confirm the code was already clean).
