Implemented the run-event append contract in `fabro-store`.

Key changes:

- Added typed [`Error::EventRejected`](/home/daytona/workspace/fabro/lib/components/fabro-store/src/error.rs:17), preserving the reducer error as its source.
- Appends now clone the current projection and run the canonical replay reducer before writing to SlateDB.
- Invalid transitions and invalid first events leave the log, caches, summary, and sequence allocation untouched.
- After a successful SlateDB write, projection caches are installed from the validated projection and SQLite summary updates are best-effort. SQLite failures emit a warning and the append still succeeds.
- Documented the three-outcome contract on all append methods in [`run_store.rs`](/home/daytona/workspace/fabro/lib/components/fabro-store/src/slate/run_store.rs:247).
- Removed the obsolete apply/rebuild/remove projection-cache path.
- Added regression coverage for rejection, reconciliation, first-event validation, sequence exhaustion, conditional appends, and repair after a forced SQLite failure.

Verification completed:

- `cargo +nightly-2026-04-14 fmt --check --all`
- `cargo +nightly-2026-04-14 clippy --workspace --all-targets -- -D warnings`
- `cargo build --workspace`
- `cargo nextest run -p fabro-store` — 241 passed
- Workspace suite excluding two known incompatible server tests — 7,542 passed, 202 skipped
- No OpenAPI or wire files changed.

The unfiltered workspace suite exposes one planned compatibility issue: these two server repair tests fail because their shared fixture explicitly depends on the old behavior of persisting an invalid event despite returning `Err`:

- `delete_run_force_removes_unreadable_durable_run`
- `delete_run_without_force_keeps_active_durable_run`

Their helper says the invalid event “should be persisted but rejected by projection.” It is now correctly rejected before writing, so the run never becomes unreadable. I left that server fixture unchanged per the scope instruction to surface callers depending on the old committed-error behavior instead of expanding this PR.

For the PR description: this fixes the vanishing-run sequence of invalid append → unreadable projection → summary deletion → run 404. Such call sites now receive `EventRejected` with nothing written. A failed SQLite update after commit now logs a warning and reports success. Existing poisoned logs are not repaired by this change.