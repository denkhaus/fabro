All review findings are addressed and the full gate is green. Summary of the simplify pass:

## Fixed

**Duplication (all three agents flagged this)**
- Extracted a shared `resolve_extra_headers(provider, headers, ctx)` helper in `fabro-auth/src/resolve.rs` — the byte-identical per-header parse/resolve/error-map blocks in `resolved_extra_headers_for_catalog` and `env_source::resolved_extra_headers` now both call it. This also hoists the `ResolveCtx` (and its boxed lookup closures) out of the per-header loop, and moves the redactor-gap comment onto the helper's doc.
- Promoted the token-only fail-closed lookup into `fabro_auth::vault_token_lookup` (`vault_ext.rs`) with a doc comment stating the wrong-schema→`None` collapse; the header resolver and `fabro-workflow/src/operations/start.rs`'s local wrapper now share the one audited implementation instead of two copies of `.ok().flatten()`.
- Factored a `portkey_catalog(extra_headers)` test fixture in both `resolve.rs` and `env_source.rs` test modules — seven tests no longer repeat the ~28-line catalog TOML (≈170 lines removed).

**Test correctness**
- Deleted two vacuous no-leak assertions in `env_source.rs` tests (`s3cr3t` / `pk-live` were never planted in those env-only tests); replaced with meaningful assertions that the error names the missing token. The real leak tests in `resolve.rs` (where values *are* planted) are untouched.
- Renamed `header_with_file_or_oauth_vault_entry_fails_closed` → `header_with_non_token_vault_entry_fails_closed` (it only exercises an OAuth entry).
- Collapsed a duplicated `auth_issues` iteration in `env_source_reports_missing_env_header_for_no_auth_provider`.

**Scoping & documentation**
- Narrowed the `#[expect(clippy::disallowed_methods)]` in `builders.rs` from the whole `provider_settings_to_catalog` function to just the `extra_headers` statement.
- Added doc comments to `ProviderCatalogSettings.extra_headers` and `CatalogProvider.extra_headers` noting they hold *unresolved* interpolation source strings resolved in `fabro-auth` — the stringly-typed boundary is now self-describing.

## Noted but intentionally skipped
- Wrong-schema vault entries reporting as "not set" instead of a schema mismatch: the plan explicitly prefers the simple fail-closed form; the collapse is now documented on `vault_token_lookup`.
- Resolve-and-discard in `configured_providers` checks: efficiency agent concluded it's not worth new API surface (tiny maps, non-hot path).
- Adding the header *key* to the `Interpolation` error variant: the interp error already names namespace+token; plan-specified shape kept.

## Verification
- `cargo +nightly-2026-04-14 fmt --check --all` ✔
- `clippy --all-targets -D warnings` on all touched crates ✔
- `cargo nextest run` for fabro-auth, fabro-config, fabro-model, fabro-llm, fabro-workflow, fabro-server: **3,009 tests passed, 0 failed** ✔
- `cargo check --workspace` ✔, `rg HeaderValueRef lib/crates` → zero hits ✔