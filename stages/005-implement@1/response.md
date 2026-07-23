Implemented the complete per-branch fidelity plan.

### Highlights
- Added ordered per-branch preamble stash generation and consumption.
- Implemented edge → branch node → fork inheritance precedence.
- Explicit branch `full` degrades to `summary:high` with structured logging.
- Preserved byte-identical fork inheritance for unspecified/equal fidelities.
- Added strict legacy fallback for missing, null, malformed, non-array, and mismatched stashes.
- Cleared branch stashes for nested-parallel safety.
- Removed stash data from checkpoints, durable snapshots, and persisted event contexts.
- Kept branch `thread_id` inert, `CURRENT_NODE` inherited, and `simulate()` unchanged.
- Updated validation rules and all three requested documentation pages.
- Added unit, lifecycle, handler, integration, persistence, and lint regression coverage.
- No changelog added.

### Verification
- `cargo nextest run --workspace`: **7,063 passed, 193 skipped**
- `cargo +nightly-2026-04-14 fmt --check --all`: passed
- `cargo +nightly-2026-04-14 clippy --workspace --all-targets -- -D warnings`: passed
- `git diff --check`: passed

Live testing-server probes were not run because the probe definitions and testing-server access were not available in this checkout/session.