Goal: # Per-Branch Fidelity for Parallel Branches — Implementation Plan

## Context

Parallel branch nodes are dispatched via `dispatch_handler`, bypassing `FidelityLifecycle::before_node` (`lib/crates/fabro-workflow/src/lifecycle/fidelity.rs:76-180`) — the only place fidelity is resolved and preambles are built. Every branch therefore inherits the stale `current.preamble` copied at `context.fork()` (`handler/parallel.rs:226`), and `fidelity`/`thread_id` on branch nodes or `fork -> branch` edges are silently ignored. Confirmed live on the testing server (probes `01KY7KRA7E…`/`01KY7KRAAW…`, 2026-07-23): a `truncate` branch received the identical compact preamble as its default sibling; fork-level fidelity works and is the current workaround. Design reviewed via the Quarry doc "Fix: Per-Branch Fidelity for Parallel Branches".

Why not run the lifecycle per branch: it is a single-token state machine (one-slot `incoming_edge_data` baton, singleton context keys written to shared run state); concurrent invocation would corrupt run state. And `build_preamble` (`handler/llm/preamble.rs:24`, public and pure) needs `state.completed_nodes`/`state.node_outcomes`, which only the lifecycle sees. So: **pre-render per-branch preambles in the lifecycle, hand off to the handler via one context key.**

## Semantics (final, after design pressure-test)

- **Explicit-only resolution.** A branch's fidelity comes from the `fork -> branch` edge attr, else the branch node attr, else **no entry** — the branch inherits the fork's preamble via `fork()` exactly as today. The fork's own resolved fidelity is never re-applied per branch; this keeps the default path byte-identical even when the fork resolves `Full` (where re-derivation would have wrongly degraded every branch).
- **`full` degrades to `summary:high`** (`Fidelity::degraded()`, `fabro-graphviz/src/fidelity.rs:35-40`) — applied only to *explicitly set* branch fidelity, with a log line (per `docs/internal/logging-strategy.md` — read before writing it).
- **Equality skip**: if the branch's post-degradation fidelity equals the fork's post-degradation fidelity, store no entry (avoid redundant renders).
- **`thread_id` stays inert in branches** (concurrent branches must never share an LLM session).
- **`CURRENT_NODE` in branch contexts stays inherited (fork id).** The pressure-test showed changing it would re-attribute every branch-internal event's stage scope (`context.rs:185-190` → `StageScope::for_handler` used by all handlers) with a visit mismatch against `for_parallel_branch` scoping. The Quarry doc's "bookkeeping keys describe the branch" line is consciously deferred to a separate change with proper visit accounting.
- **`simulate()` untouched.** No simulated handler reads preambles; partial mirroring would risk nested-parallel stash misreads. All-or-nothing → nothing.
- **Stash shape**: `Value::Array`, length = branch count, `Null` = inherit, else `{"fidelity": "...", "preamble": "..."}`. Array length ≠ edge count → treat as absent (legacy). Keyed by edge index; `graph.outgoing_edges` is an ordered Vec filter (`fabro-types/src/graph.rs:393-395`) and lifecycle + handler share the same `Arc<GvGraph>`, so indices align deterministically (including two edges to the same target).

## Implementation steps (ordered; tree compiles at each step)

1. **`lib/crates/fabro-workflow/src/context.rs`** — add `pub const INTERNAL_PARALLEL_BRANCH_PREAMBLES: &str = "internal.parallel_branch_preambles";` to `keys`. The `internal.` prefix already excludes it from preamble rendering (`preamble.rs:99-109`) and child→parent propagation (`context.rs:80-85`).

2. **`lib/crates/fabro-workflow/src/artifact.rs`** — strip the new key in `durable_context_snapshot` (`:81`) and `normalize_checkpoint_for_resume` (`:101`), beside `CURRENT_PREAMBLE`. Without this, every post-parallel checkpoint and `CheckpointCompleted` event payload carries the full per-branch preamble map (a `summary:high` preamble embeds up to 50 lines of every command output — multiplied per branch).

3. **`lib/crates/fabro-workflow/src/lifecycle/fidelity.rs`** — in `before_node`:
   - Set the stash key to `Null` on `state.context` **first**, before the two fallible `resolve_*` calls, so the always-overwritten invariant holds on every early-return path.
   - After the existing preamble build, when `gv_node.handler_type() == Some("parallel")`: iterate `self.graph.outgoing_edges(node.id())` in order; per edge resolve explicit fidelity (edge attr → target-node attr → none); apply `degraded()` to explicit values (log when it was `full`); push `Null` for inherit/equal-to-fork, else render `build_preamble(final_fidelity, …)` reusing the already-resolved snapshot (blobs resolved once at `:113-128`) and push the entry. Set the array on the stash key.
   - Extract the per-branch resolution as a pure helper beside `resolve_fidelity` (`:206`, same module — no visibility change) for unit testing.

4. **`lib/crates/fabro-workflow/src/handler/parallel.rs`** — in `execute()`'s branch-setup loop (insert after `:238`, where `INTERNAL_PARALLEL_BRANCH_ID` is set):
   - Read the stash from the parent context once before the loop; `None`, `Some(Null)`, or length-mismatch all mean strict legacy behavior (note: `Context::get` returns `Some(Null)` for a Nulled key — both must be treated as absent).
   - Per branch with an entry: `branch_context.set(CURRENT_PREAMBLE, preamble)` and `branch_context.set(INTERNAL_FIDELITY, fidelity)`. Downstream needs nothing: `agent.rs:244`/`prompt.rs:63` read `context.preamble()`.
   - In **every** branch fork, set the stash key to `Null` — load-bearing, not hygiene: a nested parallel branch target reads its fork's stash, and without the Null it would misinterpret the outer node's array as its own.
   - After the loop, set the stash key to `Null` on the handler's own context — the write-back diff (`node_handler.rs:99-105`) clears `state.context` so the post-parallel checkpoint carries Null even before the artifact strip.

5. **`lib/crates/fabro-validate/src/rules/parallel_branch_inert_attribute.rs`** — drop `"fidelity"` from `BRANCH_IGNORED_ATTRS` and its `fix_message` arm; add a narrow diagnostic in its place: `fidelity="full"` on a fork→branch edge or branch-only node warns "parallel branches run at most at summary:high; full is degraded at runtime because branches cannot share a session". Other fidelity values now lint clean. Update doc comment (the snapshot rationale now applies to `thread_id` only) and tests.

6. **`lib/crates/fabro-validate/src/rules/thread_id_requires_fidelity_full.rs`** — skip fork→branch edges and branch-only nodes (factor the branch-only detection from rule 5 into a shared helper). Today it tells branch nodes with `thread_id` to *add* `fidelity="full"` — advice that, post-change, would actively alter runtime behavior while the other rule says "remove thread_id". Defer to the inert-attribute rule's guidance on branches.

7. **Docs** — `docs/public/execution/context.mdx` (fidelity precedence: branch edge → branch node → inherit fork; per-branch preamble rendering; `thread_id` inert in branches; `full` degradation), `docs/public/workflows/stages-and-nodes.mdx` (parallel fan-out section + fidelity attribute notes), `docs/public/reference/dot-language.mdx` (edge/node attr rows). Optional changelog entry via the changelog conventions.

## Tests

Per `docs/internal/testing-strategy.md`, preamble content is implementation-facing → `fabro-workflow`, not CLI layers.

- **Pure unit tests** (`lifecycle/fidelity.rs` tests, beside `resolve_fidelity`'s at `:271-321`): explicit edge > node precedence; no-attr → inherit (no entry); explicit `full` → `summary:high` entry; branch fidelity equal to fork's (post-degradation) → no entry; fork resolved `Full` + no branch attrs → no entries at all.
- **Lifecycle-level**: two consecutive `before_node` calls on the same parallel node rebuild (not merge) the stash; non-parallel node overwrites stash to Null; resume-degrade flag interaction (fork degrades, fallback branches still get no entry).
- **`artifact.rs` tests** (`:519+` pattern): both snapshot functions strip the stash key.
- **Parallel handler unit tests** (`handler/parallel.rs` tests module, `EngineServices::test_default()` + recording handler mirroring `PreambleEchoHandler`, `manager_loop.rs:973-1043`): entry applies `CURRENT_PREAMBLE`/`INTERNAL_FIDELITY` to the right branch by index; stash Null in every branch fork; `Some(Null)`/absent/length-mismatch → legacy; duplicate-target edges get distinct entries at indices 0/1 (no-git test — a pre-existing worktree-name collision exists for that topology, don't let it pollute the assertion); existing tests stay unmodified as the legacy guard.
- **Engine-level regression** in `lib/crates/fabro-workflow/tests/it/integration.rs` beside the `fidelity_prompt_*` tests (`:9245-9479`), reusing `FidelityCapturingHandler` (`:4917-4974`) and the `end_to_end_parallel_fan_out_fan_in` scaffold (`:2441-2480`) via `WorkflowRunner::run_with_state`:
  - **Probe A analog** (`parallel_branches_get_per_branch_preambles_by_fidelity`): seed sets a context marker → fork → `branch_a` (`fidelity="truncate"`) + `branch_b` (default) → fan-in. Assert branch_a's preamble is goal-only (no marker) while branch_b's contains the marker.
  - **Probe B analog**: `fidelity="truncate"` on the fork only → both branches goal-only (compat guarantee, unchanged behavior).
  - Edge-attr-beats-node-attr variant.
- **Lint tests**: no warning for non-full branch fidelity; warning for branch `fidelity="full"`; `thread_id_requires_fidelity_full` silent on branch-only nodes, still firing elsewhere.

## Verification

- `cargo nextest run -p fabro-workflow -p fabro-validate`, then `ulimit -n 4096 && cargo nextest run --workspace` (do not export `FORCE_COLOR`).
- `cargo +nightly-2026-04-14 fmt --check --all`; `cargo +nightly-2026-04-14 clippy --workspace --all-targets -- -D warnings`.
- Live confirmation on the testing server: re-run the two probe workflows (session scratchpad `probes/isolation-a`, `probes/isolation-b`) against a locally built binary — probe A's `stage.prompt` events must now show differentiated branch preambles; probe B byte-identical to before.

## Compatibility

| Situation | Impact |
|---|---|
| No fidelity attrs near the parallel node | None — byte-identical (inherit path, no re-render) |
| Fidelity on the fork node / its incoming edge | None — fork snapshot semantics unchanged |
| Previously-dead attrs on branch nodes / fork→branch edges | Start working (the fix) |
| `full` on a branch | Degrades to `summary:high` + log + lint warning |
| `thread_id` on a branch | Still inert; lint still warns; the contradictory companion lint goes quiet on branches |

## Decisions (user-confirmed 2026-07-23)

1. **`CURRENT_NODE` in branch contexts stays inherited** — the branch-scoped bookkeeping change is deferred to a dedicated event-attribution change.
2. **The narrow `fidelity="full"` branch lint is in scope** (step 5 stands as written).
3. **No changelog entry in this PR** — changelog handled in the usual batch.


## Completed stages
- **toolchain**: succeeded
  - Script: `command -v cargo >/dev/null || { curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y && sudo ln -sf $HOME/.cargo/bin/* /usr/local/bin/; }; cargo --version 2>&1`
  - Output:
    ```
    cargo 1.95.0 (f2d3ce0bd 2026-03-21)
    ```
- **preflight_compile**: succeeded
  - Script: `cargo check -q --workspace 2>&1`
  - Output: (empty)
- **preflight_lint**: succeeded
  - Script: `cargo +nightly-2026-04-14 clippy -q --workspace --all-targets -- -D warnings 2>&1`
  - Output: (empty)
- **implement**: succeeded
  - Model: openai/gpt-5.6-sol, 394.2k tokens in / 64.5k out
  - Files: /home/daytona/workspace/fabro/docs/public/execution/context.mdx, /home/daytona/workspace/fabro/docs/public/reference/dot-language.mdx, /home/daytona/workspace/fabro/docs/public/workflows/stages-and-nodes.mdx, /home/daytona/workspace/fabro/lib/crates/fabro-validate/src/rules/mod.rs, /home/daytona/workspace/fabro/lib/crates/fabro-validate/src/rules/parallel_branch.rs, /home/daytona/workspace/fabro/lib/crates/fabro-validate/src/rules/parallel_branch_inert_attribute.rs, /home/daytona/workspace/fabro/lib/crates/fabro-validate/src/rules/thread_id_requires_fidelity_full.rs, /home/daytona/workspace/fabro/lib/crates/fabro-workflow/src/artifact.rs, /home/daytona/workspace/fabro/lib/crates/fabro-workflow/src/context.rs, /home/daytona/workspace/fabro/lib/crates/fabro-workflow/src/handler/parallel.rs, /home/daytona/workspace/fabro/lib/crates/fabro-workflow/src/lifecycle/event.rs, /home/daytona/workspace/fabro/lib/crates/fabro-workflow/src/lifecycle/fidelity.rs, /home/daytona/workspace/fabro/lib/crates/fabro-workflow/tests/it/integration.rs


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
