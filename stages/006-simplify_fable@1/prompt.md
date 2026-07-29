Goal: # PR 1 — Make run-event appends validate before write and report commit status unambiguously

**Self-contained implementation plan.** Everything needed to implement this
is in this file plus the repository.

**Precondition:** none — this is foundational work with no dependency on
other in-flight changes. Re-verify the "Verified current state" section
against HEAD before starting; if the append path in
`lib/components/fabro-store/src/slate/run_store.rs` has been materially
restructured since the pinned commit, stop and state that in the PR
description instead of adapting blindly.

> **Token notation.** Interpolation tokens are written in this file without
> their enclosing double curly braces, so the file is safe to pass directly
> as a workflow goal (the goal templater would otherwise try to expand them).
> Read `secrets.NAME`, `env.NAME`, `vars.NAME` as the double-curly-brace
> token form used in the codebase, and write the real double-brace syntax in
> the code, tests, and docs you produce.

## Context and goal

Fabro's run state is event-sourced: each run has an append-only event log in
a shared SlateDB store (`fabro-store`), a reduced in-memory projection
(`RunProjection`), and a derived SQLite summary row used by all listing
endpoints. Run status transitions are enforced by a state machine
(`RunStatus::can_transition_to` / `transition_to` in
`lib/foundation/fabro-types/src/status.rs`) — for example, a run whose
durable status is `Runnable` may legally move to `Failed` only with reason
`Cancelled`; a `Failed { WorkflowError }` from `Runnable` is an invalid
transition and the reducer hard-errors on it.

The append path has two defects, and this PR fixes both at the store layer:

**Defect 1 — poison events.** `append_event_envelope_locked` writes the
event bytes to SlateDB *before* any reduction happens. If the event turns
out to be transition-invalid, the caller gets an error — but the invalid
event is already durably in the log. From then on the run's projection can
never be rebuilt: replay hits the same invalid transition every time. The
user-visible consequence is severe: at startup, projection warmup skips the
unreadable run, and the SQLite reconciler then *deletes its summary row*
because it is absent from the authoritative entries — the run disappears
from every listing, and get/cancel return 404. This is a real shipped bug:
several server failure helpers attempt exactly such illegal appends today
(e.g. a worker-launch failure helper appends `Failed { LaunchFailed }`
while the durable status is still `Runnable`). Those call sites are being
fixed in separate planned work — this PR's job is to make the store refuse
to write the poison event in the first place.

**Defect 2 — ambiguous append errors.** After the SlateDB put succeeds, the
append still does derived work: applying the event to the shared projection
cache and upserting the SQLite summary row. Failures in either currently
propagate as `Err` from the append — so callers cannot distinguish "the
event was not committed, safe to retry" from "the event IS committed but a
derived update failed." Worse, when the projection-cache update fails, the
current code removes the cache entry entirely. Upcoming scheduler work will
retry appends that report failure, so this ambiguity must be resolved
before it exists: retrying a committed append would attempt a duplicate
event.

**Goal:** after this PR, the append contract is unambiguous:

1. An event that the current projection cannot legally reduce is **rejected
   before anything is written** — the log, the projection cache, and the
   summary row are all untouched, and the caller gets a typed rejection
   error.
2. A failure of the authoritative SlateDB put (or of event-sequence
   allocation) returns a typed **not-committed** error — safe to retry.
3. Once the authoritative put succeeds, the append **is committed** and
   reports success. Derived-state updates (projection cache install, event
   cache, SQLite summary upsert) are best-effort: failures are logged
   loudly with the run id but never surface as an append error. Derived
   state is repairable (startup reconciliation rebuilds it; the summary
   upsert is already guarded to be monotonic by event seq, so a later
   successful append also repairs it).

Design rules (fixed — do not re-litigate):

- **Validation must reuse the same reduction code that replay uses.** The
  invariant is "an event is written iff replay can reduce it." Any
  divergence between the pre-write check and replay reintroduces poison
  events. Apply the candidate event to a clone of the current projection
  using the existing reducer entry points; do not write a parallel
  validity checker.
- **No event schema changes and no public API changes.** This is a store
  contract fix, not a wire change.
- **Do not rework the failing call sites.** Server helpers that attempt
  illegal appends will now receive a clean rejection with nothing written —
  that is the intended intermediate state. Fixing their logic is separate
  planned work.
- **The rejection error must be a distinct variant** from the existing
  `Error::InvalidEvent` (which means "malformed payload") so callers can
  tell "rejected by the run's state machine" apart from "bad input" and
  from "not committed, retry."
- **Do not attempt to repair logs that already contain poison events.**
  Pre-existing corrupted logs remain unreadable and continue to be surfaced
  by the existing unreadable-runs listing; repair tooling is out of scope.

## Verified current state (as of origin/main `1aa7a153b`, 2026-07-28 — re-verify before starting)

- `lib/components/fabro-store/src/slate/run_store.rs`:
  - `append_event(&EventPayload)` → `append_event_envelope` → validates the
    payload shape (`payload.validate(&run_id)`), takes the per-run
    `state_lock`, then calls `append_event_envelope_locked` (≈ lines
    273-305).
  - `append_event_if(payload, predicate)` — same, but loads the current
    projection under the lock and returns `Ok(None)` when the predicate
    rejects (≈ 279-294). This method's contract must be preserved.
  - `append_event_envelope_locked` (≈ 305-324): allocates the event seq
    (can fail with `Error::EventSequenceExhausted`), builds the
    `EventEnvelope` (`RunEvent::try_from(payload)?`), then **puts the event
    bytes into SlateDB first**, then `cache_event`, then
    `update_summary_projection_after_append`.
  - `update_summary_projection_after_append` (≈ 325-377): applies the event
    to the shared projection cache; on failure it attempts a full rebuild
    from the db (which, for a just-written invalid event, fails again
    because the poison event is in the log), **removes the cache entry**,
    warns, and returns `Err`. If the SQLite summary store is attached
    (`run_summary_store` is an `OnceLock` — absent in some deployments),
    an upsert failure also returns `Err`. Both paths make a committed
    append look failed.
- `lib/components/fabro-store/src/error.rs`: `Error` enum with
  `InvalidEvent(String)`, `EventSequenceExhausted { max_seq }`,
  `Slate(..)`, `Sqlite(..)`, etc. No variant distinguishes
  state-machine rejection or commit status.
- `lib/foundation/fabro-types/src/status.rs` (:132-202): the transition
  table; `transition_to` returns `Err(InvalidTransition)`. From `Runnable`,
  `Failed` is legal only with reason `Cancelled`.
- `lib/foundation/fabro-types/src/run_projection.rs`: `try_apply_status`
  (≈ :1025) is where reduction enforces transitions; the reducer dispatch
  lives in `lib/components/fabro-store/src/run_state.rs`
  (`apply_event` / `apply_events`, plus `projection_from_created` for the
  first event). Both files were recently extended for new event kinds —
  re-derive exact line numbers rather than trusting the ones here.
- Startup behavior that makes poison events user-visible:
  `warm_projection_cache` in `lib/components/fabro-store/src/slate/mod.rs`
  skips runs whose replay fails (per-run `warn!`), and
  `RunSummaryStore::reconcile` deletes summary rows absent from the
  authoritative entries (pinned by the existing test
  `reconcile_removes_rows_absent_from_authoritative_entries` in
  `run_summary_store.rs`). `list_unreadable_runs` (slate/mod.rs) surfaces
  skipped runs.
- The summary upsert is monotonic by event seq (`WHERE excluded.source_last_seq > runs.source_last_seq`
  in `run_summary_store.rs`), which is what makes "later append repairs the
  row" true.
- Existing test pinning seq exhaustion:
  `append_event_rejects_sequences_beyond_key_order_limit`
  (run_store.rs ≈ :1292).

## Implementation

1. **Add the typed errors** in `lib/components/fabro-store/src/error.rs`.
   Read `docs/internal/error-handling-strategy.md` first (required by
   project convention when touching error types). Two additions, named to
   read well at call sites — suggested shapes:
   - `EventRejected { reason: String }` (or carrying the
     `InvalidTransition` detail) — the event cannot be legally reduced by
     the run's current projection; nothing was written.
   - A way for callers to know an `Err` means not-committed. Simplest
     honest contract: after this PR, **every** `Err` from append means
     not-committed (rejection included), because post-put failures no
     longer return `Err`. Prefer that global simplification over a wrapper
     enum; document it on the append methods' doc comments explicitly.
2. **Validate before the put** in `append_event_envelope_locked` (all under
   the already-held `state_lock`):
   - Obtain the current projection: the cheapest correct source is the
     same one `append_event_if` uses (`projected_state_locked`); for a run
     with no events yet, the candidate must be validated through the
     first-event path (`projection_from_created` route in
     `run_state.rs`) — mirror however `apply_events` treats the initial
     event so validation ≡ replay exactly.
   - Apply the candidate envelope to a **clone** of that projection via the
     existing reducer entry point. On reduction failure → return
     `EventRejected`, having written nothing.
   - Keep the pre-existing `payload.validate(...)` shape check where it is.
3. **Reorder the post-put work to be best-effort.** After a successful
   SlateDB put:
   - Install the already-validated clone into the shared projection cache
     (replacing the apply-then-rebuild-then-remove dance — the clone IS the
     correct post-append projection, computed before the write). Keep the
     cache's seq bookkeeping consistent with the existing
     `apply_event`/`replace` semantics.
   - `cache_event` and the SQLite upsert stay in place but become
     log-only on failure (`warn!`/`error!` with run id and seq, matching
     the logging style already present in this file). The append returns
     `Ok(envelope)` regardless of derived-state failures.
   - Do NOT remove the projection-cache entry on derived failure paths
     anymore; a stale entry that a later append or startup reconciliation
     repairs is strictly better than an absent one.
4. **Seq allocation and put failures** already return `Err` before any
   derived work — with step 3 in place these are now unambiguously
   not-committed. Verify `EventSequenceExhausted` still propagates (the
   existing test pins it).
5. **Audit append callers for compile-only impact.** Call sites that
   currently treat any `Err` as "append failed" remain correct under the
   new contract (their errors now genuinely mean not-committed). No caller
   behavior changes in this PR. `append_event_if`'s `Ok(None)` predicate
   contract is unchanged.
6. **Doc comments.** State the three-outcome contract (rejected-nothing-
   written / not-committed / committed-with-best-effort-derived) on
   `append_event`, `append_event_if`, and `append_event_envelope`.

## Scope boundaries — deliberately NOT in this PR

- **The server failure helpers that attempt illegal appends** (e.g. the
  worker-launch failure path appending `Failed { LaunchFailed }` from
  durable `Runnable`, and similar pre-worker failure sites in
  `fabro-server`) — leave their logic as-is. They will now receive a clean
  `EventRejected` and write nothing, which is the intended intermediate
  state; reworking when/what they append is separate planned work. Do not
  "fix" them to append legal events.
- **Admission/scheduler changes** (durable claims, retry/backoff, startup
  re-admission of queued runs) — known follow-up work, deliberately
  excluded here.
- **Repairing already-poisoned logs** or adding repair/diagnostic tooling —
  known gap, addressed separately if needed. Pre-existing unreadable runs
  keep their current behavior (skipped at warmup, surfaced by the
  unreadable-runs listing).
- **Event schema, OpenAPI, or public API changes** — none. This PR is
  entirely inside `fabro-store` (plus its error type).
- **SQLite schema changes** — none; the monotonic upsert and startup
  reconcile already provide the repair path.

If work outside these boundaries seems genuinely required for this PR to
compile or pass its tests, stop and state that in the PR description rather
than expanding scope.

## Tests (write failing-first; hermetic — temp-dir fixtures, no ambient provider keys)

Existing store tests in `run_store.rs` / `run_summary_store.rs` show the
fixture style (temp-dir object store, in-memory SQLite). Add:

1. **Rejected transition writes nothing** — create a run, drive it to
   durable `Runnable` (append the events the lifecycle uses today:
   created/submitted/start-requested/runnable), then append a
   `run.failed { WorkflowError }`-shaped event. Assert: the append returns
   the rejection variant; `list_events` shows no new event; `state()` still
   reduces successfully; the projection cache still holds an entry for the
   run (not removed). *Property pinned: an event is written iff replay can
   reduce it.*
2. **Rejected transition leaves listings consistent** — after the rejected
   append, run the summary reconcile path and assert the run's summary row
   still exists. *Property: no more vanishing runs from rejected appends.*
3. **Committed append survives derived-state failure** — attach a SQLite
   summary store, then make its pool unusable (e.g. close the pool or drop
   the underlying file) before appending a legal event. Assert: append
   returns `Ok`; the event is in `list_events`; a warning/error was the
   only symptom. Then restore/reopen the summary store and assert the row
   is repairable (via reconcile or a subsequent append). If pool-closing
   proves impractical through public seams, an injected failing summary
   store behind the existing test-support feature is acceptable — but do
   not weaken the assertion that append reports success. *Property:
   committed is committed.*
4. **Not-committed errors are retryable** — the existing
   seq-exhaustion test keeps passing; extend it (or add a sibling) to
   assert the log is unchanged after the error, pinning "Err ⇒ nothing
   written."
5. **First-event validation** — a malformed first event (one the reducer
   cannot initialize a projection from) is rejected with nothing written;
   a valid `run.created` still works. *Property: the empty-log path
   validates like replay too.*
6. **append_event_if contract unchanged** — predicate-false still returns
   `Ok(None)` with nothing written.

Run the full workspace suite; the reducer and lifecycle tests in
`fabro-store`, `fabro-workflow`, and `fabro-server` are the regression net
for "legal appends behave exactly as before."

## Acceptance / verification

- `cargo +nightly-2026-04-14 fmt --check --all`
- `cargo +nightly-2026-04-14 clippy --workspace --all-targets -- -D warnings`
- `cargo nextest run --workspace`
- No OpenAPI/wire change (do not touch `docs/public/api-reference/`).
- `cargo build --workspace` without the `test-support` feature still
  succeeds if any test helper was added behind it.

## Conventions

- Read `docs/internal/error-handling-strategy.md` before changing the error
  enum, and `docs/internal/events-strategy.md` before touching anything
  that emits or documents events.
- Never print or log a resolved secret value, including from tests.
- Plain-English commit messages, PR text, and comments — describe what the
  change does; no internal planning identifiers or plan-file names in
  anything that ships.
- PR description must state plainly: (1) the vanishing-runs failure mode
  this fixes (invalid append → unreadable projection → summary row deleted
  → run 404s) and that call sites attempting such appends now get a clean
  error with nothing written; (2) the new append contract, including that
  a failed SQLite summary update after a committed append now logs loudly
  and reports success instead of returning an error — operators see a
  warning where they previously saw a failed operation; (3) that
  pre-existing corrupted run logs are not repaired by this change.
- If implementation uncovers a caller that genuinely depends on the old
  "Err after committed write" behavior, stop and surface it in the PR
  description rather than working around it.


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
  - Model: gpt-5.6-sol
  - Files: /home/daytona/workspace/fabro/lib/components/fabro-store/src/error.rs, /home/daytona/workspace/fabro/lib/components/fabro-store/src/run_summary_store.rs, /home/daytona/workspace/fabro/lib/components/fabro-store/src/slate/mod.rs, /home/daytona/workspace/fabro/lib/components/fabro-store/src/slate/projection_cache.rs, /home/daytona/workspace/fabro/lib/components/fabro-store/src/slate/run_store.rs, /home/daytona/workspace/fabro/lib/components/fabro-store/src/types.rs


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
