Cleanup pass complete.

Fixed:
- Removed the new public `fabro_auth::vault_token_lookup` re-export; kept the helper `pub(crate)` inside `fabro-auth` to avoid widening the public API unnecessarily.
- Restored `fabro-workflow`’s private token-only vault lookup to call the already-public `vault_get_token(...).ok().flatten()`.
- Simplified OAuth credential construction by removing an unnecessary `std::mem::take`.
- Reduced duplicated test TOML by reusing the existing `portkey_catalog(...)` test helper in `fabro-auth/src/resolve.rs`.

Verified:
- `cargo +nightly-2026-04-14 fmt --check --all`
- `cargo nextest run -p fabro-auth`
- `cargo +nightly-2026-04-14 clippy -q -p fabro-auth -p fabro-workflow --all-targets -- -D warnings`
- `cargo dev docs check`
- `rg -n "HeaderValueRef" lib/crates` returns no hits

I did not run the full workspace nextest/clippy/build/typecheck gate in this cleanup pass.