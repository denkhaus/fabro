The code is clean and all checks pass. Here's a summary of what the review found and what I fixed.

## Review outcome

Three parallel agents (reuse, quality, efficiency) converged on the same core issues. I fixed all high-value ones:

**1. Extracted a shared region helper (reuse + quality).**
The sort → merge-overlaps → splice-with-`REDACTED` loop was duplicated between `redact_string` (lib.rs) and the new module. I pulled it into `pub(crate) fn redact_regions(s, Vec<Region>)` in lib.rs; both `redact_string` and the registry now call it.

**2. Deleted per-call clone + sort on the hot path (efficiency + quality).**
The original `registered_values_longest_first` cloned the entire `Vec<String>` and re-sorted it on *every* `redact_into`/`redact_json` call. By reusing `redact_string`'s **merge** overlap strategy, longest-first ordering is no longer needed for correctness, so that method, the O(n²) `regions_overlap` drop scan, and all per-call sorting/cloning are gone. Redaction now just holds the read guard, collects match regions, and delegates.

**3. Merge is also strictly safer.** The old drop-overlapping-regions logic could leak a partial-secret tail when two registered secrets partially overlapped in the text; the merge strategy redacts the union.

**4. Empty fast-path (efficiency, plan-required).** `redact_into`/`redact_json` now check `is_empty()` first and return the input untouched before allocating anything.

**5. Centralized lock/poison handling (quality).** Added private `read()`/`write()` guard helpers so `unwrap_or_else(PoisonError::into_inner)` lives in one place instead of three, and simplified dedup to `Vec::contains`.

**6. Renamed** the private `redact_json_value` → `redact_json_leaves` to avoid colliding with the public `jsonl::redact_json_value`.

Not changed (false positives / intentional): keeping `RwLock` (now justified since redaction holds the read guard without cloning), the small-N per-value scan (documented as an assumption), and the owned-`Value` signature (matches existing `redact_json_value`).

Verification: `fmt --check`, `clippy --all-targets -D warnings`, and `cargo nextest run -p fabro-redact` (66 tests) all pass.