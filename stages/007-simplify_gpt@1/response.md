Reviewed the sandbox glob changes and made two cleanup fixes:

- Reused a single `MATCH_OPTIONS` constant in `glob_match.rs` instead of reconstructing `glob::MatchOptions` on every match.
- Tightened Local traversal to use `symlink_metadata()` so `LocalSandbox::glob` does not recurse through symlinked directories or escape/cycle unexpectedly.
  - Added a Unix regression test for symlinked directory traversal.

Validation run:

- `cargo +nightly-2026-04-14 fmt --check --all`
- `cargo test -q -p fabro-sandbox glob --lib`
- `cargo check -q -p fabro-sandbox`
- `cargo +nightly-2026-04-14 clippy -q -p fabro-sandbox --all-targets -- -D warnings`

All passed. Current working tree has the two cleanup edits in:

- `lib/crates/fabro-sandbox/src/glob_match.rs`
- `lib/crates/fabro-sandbox/src/local.rs`