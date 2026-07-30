The verify failure is fixed — the entire pipeline now passes end to end. No code changes were needed.

## Root cause

The failure was environmental, not a code defect. The disk was 100% full (`20G/20G` used), so `bun install --frozen-lockfile` failed with `ENOSPC`/`NoSpaceLeft` while extracting packages, and it also left `node_modules` in a broken half-linked state. The space was consumed by 18 GB of accumulated debug artifacts in `target/debug/deps` (many stale duplicate compilation units from repeated workflow stages) plus ~120 MB of leftover `/tmp/fabro-test-*` fixtures.

## What I did

1. Deleted `target/debug`, stale `/tmp/fabro-test-*` dirs, and the corrupted `node_modules` trees (freed ~18 GB).
2. Re-ran every step of the verify script in order:
   - `git fetch` / `git merge origin/main` — already up to date.
   - `cargo +nightly-2026-04-14 fmt --all` then `fmt --check --all` — clean.
   - `cargo dev docs refresh` + `cargo dev docs check` — both generated docs up to date, no diff.
   - Forbidden-pattern `rg` check — passed (note: the script's `lib/crates` path doesn't exist in the repo; `rg` warns but the check still evaluates correctly since no forbidden patterns match).
   - `cargo +nightly-2026-04-14 clippy --workspace --all-targets -- -D warnings` — clean.
   - `cargo nextest run --workspace --profile ci` — **7658 tests passed, 0 failed** (203 skipped), including the new fabro-store append-contract tests.
   - `bun install --frozen-lockfile` — 1625 packages installed successfully.
   - `apps/fabro-web`: typecheck clean, **799 tests passed, 0 failed**.
   - `lib/packages/fabro-api-client`: typecheck clean.
   - `cargo dev build -- -p fabro-cli --release` — succeeded, SPA asset budgets verified.

The working tree is clean (no uncommitted changes — nothing in the PR's code needed modification), and the disk now has 5.7 GB of headroom after the release build.