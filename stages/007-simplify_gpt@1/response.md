Cleanup review complete.

Fixed in `lib/crates/fabro-redact/src/secret_registry.rs`:

- Released the registry read lock before doing string/JSON redaction by snapshotting registered values first.
- Preserved an explicit empty-registry fast path.
- Added a no-match short-circuit in `redact_string_values` to avoid calling `redact_regions` unnecessarily.
- Clarified docs:
  - Registered values are exact substring matches and may be low-entropy.
  - JSON redaction affects string values only; object keys and non-string values are unchanged.

Skipped/left unchanged:

- Did not add minimum length / entropy filtering because the plan explicitly requires low-entropy values like `"staging"` and overlapping short values like `"abc"`.
- Did not add Aho-Corasick or `IndexSet`; current registry is intentionally small per run, and the simpler implementation matches the plan.

Verification passed:

- `cargo +nightly-2026-04-14 fmt --check --all`
- `cargo nextest run -p fabro-redact`
- `cargo +nightly-2026-04-14 clippy -q -p fabro-redact --all-targets -- -D warnings`
- `cargo dev build -- -p fabro-cli --release`

Working tree currently has only:

- `M lib/crates/fabro-redact/src/secret_registry.rs`