All three review agents completed, and I fixed the aggregated findings. Full verification passes: `fmt --check`, workspace clippy with `-D warnings`, `cargo build --workspace` (without `test-support`), and `cargo nextest run --workspace` (7655 passed).

## Fixed

**Correctness / hidden regression found by the full-suite run**
- Two `fabro-server` tests (`delete_run_force_removes_unreadable_durable_run`, `delete_run_without_force_keeps_active_durable_run`) were broken by the implement stage: their fixture `create_unreadable_durable_run` relied on the old poison-write behavior (append persists the invalid event, then errors). Since appends now refuse to write, the fixture creates the poison event via a new `Database::test_put_unvalidated_run_event` helper gated behind `#[cfg(any(test, feature = "test-support"))]`, consumed through Cargo's dev-dependency dual-listing (`fabro-store` with `test-support` in `fabro-server`'s dev-deps only). This correctly simulates the scenario those tests exercise — a *pre-existing* corrupted log, which this PR deliberately does not repair.

**Reuse**
- The new pre-write validation hand-rolled a Some/None reduction dispatch that duplicated `apply_cached_projection_event` 250 lines below in the same file. It now calls that helper, so validation and replay share one literal implementation (`Arc::make_mut` copy-on-write keeps the cache untouched on rejection).
- Deleted `RunEvent::from_ref` in `fabro-types` — dead after the switch to `from_value`, and its lenient field coercion is precisely the append/read divergence this PR closes.
- Two pre-existing store tests that raw-wrote poison events now use the new helper; the fresh-writer hydrate test reuses `workflow_failure_payload`; the repair test reuses a new `test_util::sqlite_summary_store_at` instead of re-deriving the SQLite path.

**Quality**
- `Error::InvalidTransition` rendered "invalid status transition:" twice (the wrapper template plus the inner Display). Changed to `#[error(transparent)]`, which let `event_rejected`'s special-case match collapse to a single conversion.
- `warn!` on summary-upsert failure now uses `error = %err` (Display, per the logging strategy) and message casing matching its neighbors; the `Box::pin` comment again names the `clippy::large_futures` constraint it exists for.

**Efficiency**
- Every append (and `append_event_if` twice) issued a SlateDB scan under `state_lock` that is provably empty in steady state, since `state_lock` serializes appends and the local cache is always current afterward. Added `projected_state_for_append_locked`, which validates against the cached projection when `last_seq + 1 == seq` and falls back to the scan only on a cold cache.

**Noted, deliberately skipped**: carrying `InvalidTransition` typed inside `EventRejected` (plan specifies `reason: String`; revisit if a caller needs 409-vs-500 mapping), moving seq allocation after validation (rejected appends burn a seq, but exhaustion needs ~4B rejections and reopen reclaims gaps — pre-existing behavior), and reordering the summary upsert before the cache install to save one `Run` clone (worse ordering: slow SQLite would delay cache freshness).