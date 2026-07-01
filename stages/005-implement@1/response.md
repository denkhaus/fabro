Implemented the `Sandbox::glob` consistency fix.

### What changed

- Added shared glob matching in `lib/crates/fabro-sandbox/src/glob_match.rs`
  - Uses `glob::Pattern`
  - Uses `require_literal_separator: true`
  - Builds the same full pattern shape as Local previously used
  - Derives a literal traversal root to avoid listing more than needed

- Updated providers:
  - **Local**: now enumerates files under the traversal root and filters through shared Rust matching; preserves newest-first mtime sorting.
  - **Docker**: now runs `find <root> -type f` only for traversal, sorts candidates in Rust, then filters through shared matching.
  - **Daytona**: now recursively lists files via Daytona filesystem API, sorts candidates, then filters through shared matching.

- Added regression coverage for:
  - `*/SKILL.md` matching exactly one directory level
  - `*.rs` top-level-only matching
  - `**/*.rs` recursive matching
  - Shared matcher parity with `glob::glob` on fixtures
  - Local provider relative path resolution
  - Local provider skill discovery symptom
  - Local provider recursive glob behavior

### Verification run

All passed:

- `cargo test -p fabro-sandbox glob_ --lib`
- `cargo check -p fabro-sandbox --features docker,daytona`
- `cargo check -q --workspace`
- `cargo +nightly-2026-04-14 fmt --check --all`
- `cargo +nightly-2026-04-14 clippy -q --workspace --all-targets -- -D warnings`
- `cargo nextest run --workspace` — 6918 passed, 185 skipped
- `cd apps/fabro-web && bun run typecheck && bun test` — 650 passed
- `cargo doc --workspace --no-deps` — passed, with unrelated existing rustdoc warnings
- `cargo build --workspace --release`

No git commit was created.