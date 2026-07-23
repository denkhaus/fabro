The verify failure was **not a code problem** — it was a full disk. The build output showed `ld terminated with signal 7 [Bus error]` and `rustc-LLVM ERROR: IO failure on output stream: No space left on device`: the 20G disk was at 100%, with 19G consumed by `target/debug` (accumulated test binaries from prior stages) plus ~900M of stale `/tmp/fabro-test-*` run stores.

**What I did:**

1. **Freed ~13G of disk space** — removed a 194M partial linker temp file (`fabro-*.tmp*`), stale `/tmp/fabro-test-*` directories, and relinkable debug artifacts (test executables, workspace rlibs, examples). No source or config changes were needed.

2. **Re-ran the full verify sequence** — all steps now pass with zero code changes:
   - `git fetch/merge origin/main` — already up to date
   - `cargo +nightly-2026-04-14 fmt --all` + `fmt --check` — clean
   - `cargo dev docs refresh` / `docs check` — up to date, no drift
   - forbidden-pattern `rg` check — clean
   - `cargo +nightly-2026-04-14 clippy --workspace --all-targets -- -D warnings` — clean
   - `cargo nextest run --workspace --profile ci` — **7113 passed, 0 failed**, 193 skipped
   - `bun install --frozen-lockfile`, fabro-web typecheck + tests (**680 pass**), fabro-api-client typecheck — clean
   - `cargo dev build -- -p fabro-cli --release` — succeeded, binary at `target/release/fabro`

3. **Pruned large debug test executables again before the release build** so thin-LTO linking had headroom; disk finished at 67% used with 6.8G free.

The working tree is clean (no uncommitted changes) — the per-branch fidelity implementation from the earlier stages verified successfully as-is. If the verify stage is re-run by the pipeline, note the rebuild will regrow `target/debug`; the disk now has enough headroom for one full cycle, but this sandbox's 20G disk is tight for `clippy --all-targets` + full nextest + a release build in one script.