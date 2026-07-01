Implemented Plan A in `fabro-redact`.

Summary:
- Added `SecretRedactor` in `lib/crates/fabro-redact/src/secret_registry.rs`.
- Exported it from `lib/crates/fabro-redact/src/lib.rs`.
- Reused the crate’s shared `REDACTION_MARKER` constant.
- Implemented:
  - shared cloneable per-run state via `Arc<RwLock<Vec<String>>>`
  - `register`, ignoring empty/whitespace-only values and de-duplicating
  - `redact_into`, using longest-first exact-match redaction
  - `redact_json`, redacting all JSON string leaves while preserving object keys
  - `is_empty`
- Added unit tests covering low-entropy values, empty values, overlaps, empty identity behavior, nested JSON, and shared clone state.

TDD:
- Red step confirmed: `cargo nextest run -p fabro-redact` failed before implementation because `SecretRedactor` was missing.
- Green step passed after implementation.

Verification run:
- `cargo +nightly-2026-04-14 fmt --check --all`
- `cargo +nightly-2026-04-14 clippy --workspace --all-targets -- -D warnings`
- `cargo nextest run -p fabro-redact`
- `cargo dev build -- -p fabro-cli --release`
- `cargo dev docs check`
- `cd apps/fabro-web && bun run typecheck`
- `cd lib/packages/fabro-api-client && bun run typecheck`
- `cargo nextest run --workspace`

All passed. Note: the first release build attempt failed because Bun workspace dependencies were not installed; I ran `bun install --frozen-lockfile` and reran successfully. No tracked files were changed by that install.