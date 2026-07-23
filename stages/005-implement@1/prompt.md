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


Read the plan file referenced in the goal and implement every step. Make all the code changes described in the plan. Use red/green TDD.