Done. The previous implement stage had failed on an API auth error before doing any work, so I completed the implementation and then ran the three-agent review.

## What was done

**Implementation** (per the plan):
- Deleted `lib/crates/fabro-redact/src/secret_registry.rs` (217 lines: the never-consumed `SecretRedactor` and its tests), plus the `mod` declaration and `pub use` re-export in `lib.rs`
- Rewrote the `resolve_extra_headers` doc comment in `lib/crates/fabro-auth/src/resolve.rs:478-483` — keeps the "never log resolved header values" guidance, drops the abandoned exact-match-registration promise, and states the residual risk (low-entropy header values aren't caught by content-based detection)
- Rewrote the `interp.rs` module doc in `lib/crates/fabro-types/src/settings/interp.rs:17-20` to describe the real architecture: resolved secrets are plain strings, redaction is content-based at output-serialization surfaces

**Review findings** (three parallel agents — reuse, quality, efficiency):
- All three verdicts: clean. Deletion is complete (zero `SecretRedactor` references remain; the `secret_registry` module in `fabro-static` is an unrelated registry of secret env-var *names*); nothing in `fabro-redact` became dead (`Region`, `redact_regions`, `serde_json`, and all deps still have live users); no runtime code paths changed; no dependent crate lost its reason to depend on `fabro-redact`
- **Fixed** two wording nits from the quality reviewer: clarified the ambiguous "nothing substitutes these exact values" phrasing in `resolve.rs`, and removed the "resolved … resolved" redundancy in `interp.rs`
- **Skipped** one finding: `Region` could be `pub(crate)` — pre-existing and the plan explicitly says leave `Region` untouched

**Verification** — all green:
- `cargo +nightly-2026-04-14 fmt --check --all` ✓
- `cargo +nightly-2026-04-14 clippy --workspace --all-targets -- -D warnings` ✓
- `cargo nextest run --workspace` — 6955 passed, 0 failed ✓
- Straggler sweep clean (only the unrelated `fabro-static` name registry remains)