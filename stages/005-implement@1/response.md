Implemented the `llm.providers.<id>.extra_headers.*` migration from `HeaderValueRef` to interpolation strings.

What changed:
- `fabro-config` authoring surface now uses `Option<HashMap<String, InterpString>>`.
- `fabro-model` catalog now stores provider `extra_headers` as source `String` values and `HeaderValueRef` was removed from Rust code/re-exports.
- Config → catalog conversion collapses `InterpString` to source strings with an audited `as_source()` use.
- `fabro-auth` now resolves provider headers through `InterpString`:
  - env-only source resolves `{{ env.NAME }}` and fails closed on `{{ secrets.NAME }}`.
  - vault-backed resolver resolves `{{ env.NAME }}` and token-only `{{ secrets.NAME }}`.
  - `vars.*` / `inputs.*` are out of scope and fail through the interpolation context.
  - New typed `ResolveError::Interpolation` preserves the interpolation error source.
- Header values can now combine literals and tokens, e.g. `Bearer {{ secrets.TOKEN }}`.
- `base_url` remains unchanged as a plain literal string.
- Updated tests, docs examples, generated user configuration reference, OpenRouter examples, and added `docs/public/changelog/2026-07-09.mdx`.
- Did not implement the optional credential-shaped bare-literal warning lint because the plan marked it off by default.

TDD / verification:
- Confirmed red first with targeted `cargo nextest` against the new tests, which failed on the old `HeaderValueRef` types.
- Passed:
  - `cargo check -q --workspace`
  - targeted `cargo nextest` for config/auth/LLM header behavior
  - `cargo +nightly-2026-04-14 fmt --check --all`
  - `cargo +nightly-2026-04-14 clippy --workspace --all-targets -- -D warnings`
  - `cargo dev docs check`
  - `cargo nextest run --workspace` with common provider API-key env vars stripped
  - `cd apps/fabro-web && bun run typecheck`
  - `cd lib/packages/fabro-api-client && bun run typecheck`
  - `cargo dev build -- -p fabro-cli --release`
  - `rg -n "HeaderValueRef" lib/crates` returned no hits
  - `git diff --check`

Note: I ran `bun install` because the TypeScript typecheck commands initially failed due to `tsc` not being installed in the workspace.