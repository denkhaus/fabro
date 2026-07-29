Implemented the run-event append contract in `fabro-store`.

Key changes:

- Added typed `Error::EventRejected` in [error.rs](/home/daytona/workspace/fabro/lib/components/fabro-store/src/error.rs:16).
- Appends now clone and reduce the current projection before writing in [run_store.rs](/home/daytona/workspace/fabro/lib/components/fabro-store/src/slate/run_store.rs:320).
- First events use the same reducer initialization path as replay.
- Event decoding now uses replay’s strict parser, preventing malformed optional envelope fields from being accepted during append but rejected during replay.
- After a successful SlateDB put, the validated projection is installed directly into both caches. SQLite summary failures now emit a structured warning and return success.
- Removed the old apply/rebuild/remove cache failure path.
- Documented the three-outcome append contract on all append methods.
- Added regression coverage in [slate/mod.rs](/home/daytona/workspace/fabro/lib/components/fabro-store/src/slate/mod.rs:900) for rejected transitions, reconciled listings, derived-state failure, first-event validation, strict payload decoding, and repairability.
- Extended sequence-exhaustion coverage to assert that errors leave the log unchanged.
- No OpenAPI, event schema, SQLite schema, or server call-site changes.

Verification:

- `cargo +nightly-2026-04-14 fmt --check --all` — passed
- `cargo +nightly-2026-04-14 clippy --workspace --all-targets -- -D warnings` — passed
- `cargo build --workspace` — passed
- `cargo nextest run -p fabro-store` — 242 passed
- Workspace suite excluding two known incompatible server fixtures — 7,653 passed

The unfiltered workspace suite has exactly two failures:

- `delete_run_force_removes_unreadable_durable_run`
- `delete_run_without_force_keeps_active_durable_run`

Their shared server fixture deliberately creates an unreadable run by asserting that a reducer-invalid event is persisted despite append returning an error. This PR intentionally makes that impossible: the event is now rejected without being written, so the fixture no longer creates an unreadable run. Per the scope boundary, I left those server tests and their poison-log fixture unchanged rather than expanding this PR outside `fabro-store`.

Pre-existing corrupted logs remain untouched and continue to be handled by the existing unreadable-runs path.