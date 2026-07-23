Everything is green. Summary of the simplify stage:

## Review findings applied

**Wire-format root cause (reuse #1, quality #1/#2, efficiency #2/#3):**
- Added `Serialize`/`Deserialize` to `fabro_graphviz::Fidelity` (serde renames aligned with strum per CLAUDE.md, plus `Hash` and a serde↔strum parity test; `serde_json` added as dev-dependency).
- Replaced the hand-rolled JSON producer/parser pair with one shared `ParallelBranchPreamble` struct in `context.rs` (`deny_unknown_fields` replaces the brittle `entry.len() == 2` check). Producer uses `serde_json::to_value`, consumer uses `from_value` — a field rename can no longer silently split the contract. Documented the index-correlation invariant and the `Option<Option<…>>` semantics (quality #5/#8).

**Duplication (reuse #2/#3, quality #6/#7):**
- Extracted `explicit_fidelity()` — the edge→node precedence core is now shared between `resolve_fidelity` and `resolve_parallel_branch_fidelity`.
- Added `keys::TRANSIENT_CONTEXT_KEYS` + `artifact::strip_transient_keys()`; both artifact strip sites use it. `stage_context_values` keeps its intentionally different behavior (retains `CURRENT_PREAMBLE`) with a WHY comment.
- Unified the two copy-pasted full-fidelity diagnostic blocks into `full_fidelity_diagnostic()`; `full_fidelity_message()` → `const`; dead `fix_message` match → `const THREAD_ID_FIX`; capitalized the message for consistency; reworded the "now honors" changelog-style doc comment.

**Structure & style (quality #3/#4/#9/#10/#14):**
- Extracted the 38-line parallel stash block into `build_parallel_branch_preambles()`, restoring `before_node`'s numbered-step narrative (steps renumbered 5–9).
- `ParallelBranchFidelityResolution`: dropped accessor ceremony and unused derives; plain documented fields.
- Fixed the warn! to log `effective_fidelity` via `degraded()` instead of dressed-up constants; removed the redundant `fidelity` field.
- `parallel.rs` uses `keys::`-scoped types; test helper `fidelity_attr` → `str_attr`.

**Efficiency (#1, #4):**
- Moved the parent stash clear to *before* the fork loop — branch forks no longer copy the full preamble array B times just to null it, and the per-branch null-set became unnecessary (forks inherit Null; nested-parallel safety preserved, verified by the existing `stash == Null` test assertions).
- Memoized per-fidelity preamble renders — duplicate branch fidelities render once.

**Validate helper (reuse #4, quality #13, efficiency #6):**
- `ParallelBranches` now uses `Graph::incoming_edges`; `is_branch_only_node` is allocation-free; `branch_only_parents` replaced the dense peekable/`then`/`collect::<Option<BTreeSet>>` chain with straightforward code. Added the cross-rule suppression WHY comment in `thread_id_requires_fidelity_full`.

**Skipped (noted, not addressed):** test-fixture consolidation into `rules/test_support` and a shared in-memory-store test helper (pre-existing pattern across ~15 sites, out of scope); `"parallel"` string literal (codebase-wide convention); `make_run_options` parameterization (follows existing file precedent).

## Verification
- `cargo nextest run -p fabro-graphviz -p fabro-validate -p fabro-workflow` — all pass (302 + 1218)
- `cargo nextest run --workspace` — **7064 passed, 0 failed**
- `cargo +nightly-2026-04-14 fmt --check --all` — clean
- `cargo +nightly-2026-04-14 clippy --workspace --all-targets -- -D warnings` — clean

No blockers.