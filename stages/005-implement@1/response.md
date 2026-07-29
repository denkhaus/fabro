Implemented the source-neutral run compiler boundary and rewired run creation to use it.

Key changes:

- Added the typed compiler stages in [run_compiler.rs](/home/daytona/workspace/fabro/lib/apps/fabro-server/src/run_compiler.rs:41): source normalization, settings/variables and graph compilation, model materialization, and pure persistence-input assembly.
- Kept all HTTP and `fabro_api` request types outside the boundary.
- Split the workflow create pipeline in [create.rs](/home/daytona/workspace/fabro/lib/components/fabro-workflow/src/operations/create.rs:295), including a persist-precompiled entry point. Existing `operations::create` remains as the composed compatibility API.
- Rewired `create_run_from_manifest` as the transport adapter while preserving policy checks, logging, error mappings, automation callers, and post-create side effects.
- Removed `run_manifest::create_run_input` and all post-assembly input mutation.
- Added pinning coverage in [server/tests.rs](/home/daytona/workspace/fabro/lib/apps/fabro-server/src/server/tests.rs:3664), stage-level compiler tests, mismatched-bundle protection, source/config snapshot tests, and no-recompile persistence coverage.
- No OpenAPI or `fabro-store` files were changed.

All verification passed:

- `cargo +nightly-2026-04-14 fmt --check --all`
- `cargo +nightly-2026-04-14 clippy --workspace --all-targets -- -D warnings`
- `cargo build --workspace`
- `cargo nextest run --workspace` — 7,664 passed, 203 skipped
- `git diff --check`