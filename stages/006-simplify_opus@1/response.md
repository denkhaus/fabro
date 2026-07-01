The code is clean and all gates pass. Here's a summary of the review and cleanup.

## Summary

I ran `git diff HEAD~1 HEAD`, launched three parallel review agents (reuse, quality, efficiency), and reconciled their findings.

### Fixed
- **Redundant allocations + unreachable fallback in `local.rs` glob** (efficiency + quality): It cloned every candidate path into a `Vec`, built a whole-tree `HashMap<path, mtime>`, matched, then looked mtimes back up with an unreachable `.unwrap_or(UNIX_EPOCH)`. Now it filters the `(path, SystemTime)` pairs from traversal directly, sorts by the mtime already in hand, and maps to paths — no extra clones, no whole-tree map, no dead fallback.
- **Over-abstracted `GlobMatcher` + copy-pasted match tail** (quality): The single-field struct was constructed-and-immediately-consumed in one place, and the `sort + match_glob` tail was duplicated across all three providers. I made `GlobMatcher` (with a `matches(&str)` method) the single shared matcher used by Local, Docker, and Daytona, and removed the redundant `match_glob` free function. Docker/Daytona now filter with the matcher and sort only the matches (not the full candidate list). A small test-only `match_glob` helper keeps the existing semantics tests unchanged.
- This also resolved a **`dead_code` regression**: because Local moved off the free function, `match_glob` would have been unused under the default `local`-only feature build and failed clippy `-D warnings`. Consolidating onto `GlobMatcher` (used by all three providers) removes that entirely — no `#[cfg]` hacks.

### Reviewed and deliberately skipped
- **Parity test "coincidental" (Agent 2)** — false positive. `glob::glob()` is traversal-based so `*` never crosses `/`; `require_literal_separator: true` is exactly what makes `Pattern::matches_with` reproduce that. The test is valid.
- **Sort-order inconsistency across providers (Agent 2)** — Local=mtime, remote=lexical *preserves each provider's prior behavior*, which the plan sanctioned. Changing it risks the `Glob` tool's caller expectations.
- **Daytona serial-RTT walk / depth pruning (Agent 3)** — the plan explicitly marked these as optional follow-ups; implementing them now is scope creep.
- **Path-helper consolidation — `is_absolute`/`parent_path`/`build_path`/`join_path` vs `std::path` and `resolve_path` (Agent 1)** — reworking these risks cross-provider behavior changes for marginal gain; the current code is tested and clear.

### Verification
- `cargo check` (workspace, and sandbox with `docker,daytona`): pass
- `cargo +nightly-2026-04-14 clippy -p fabro-sandbox --all-targets --all-features -- -D warnings`: pass
- `cargo nextest run -p fabro-sandbox --all-features`: 202 passed, 5 skipped
- `cargo +nightly-2026-04-14 fmt --check -p fabro-sandbox`: clean