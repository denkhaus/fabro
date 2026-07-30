Review and cleanup are complete. Run-event appends now have an unambiguous commit contract:

- Invalid transitions are reduced against a cloned projection and rejected before SlateDB is touched.
- Rejections preserve their typed source through `EventRejected`, and rejected attempts no longer consume sequence numbers.
- After the SlateDB write, the validated projection is installed directly. SQLite summary failures warn with the run ID and sequence but still report append success.
- SQLite work now runs outside the per-run state lock.
- The repair test exercises the real startup warmup/reconciliation path.
- Raw corruption injection is isolated behind the feature-gated [test support module](/home/daytona/workspace/fabro/lib/components/fabro-store/src/test_support/mod.rs:10).
- The existing borrowed `RunEvent` decoder was restored, avoiding an accidental public API removal.

The core implementation is in [run_store.rs](/home/daytona/workspace/fabro/lib/components/fabro-store/src/slate/run_store.rs:288), with the typed rejection in [error.rs](/home/daytona/workspace/fabro/lib/components/fabro-store/src/error.rs:15) and regression coverage in [slate/mod.rs](/home/daytona/workspace/fabro/lib/components/fabro-store/src/slate/mod.rs:919).

This prevents the vanishing-run failure mode: an invalid append no longer poisons replay, so reconciliation does not delete the run summary. Existing corrupted logs remain unchanged and repair tooling remains out of scope. No caller was found relying on the old “error after committed write” behavior, and no OpenAPI files were changed.

Validation passed:

- `cargo +nightly-2026-04-14 fmt --check --all`
- `cargo +nightly-2026-04-14 clippy --workspace --all-targets -- -D warnings`
- `cargo build --workspace`
- `cargo check -p fabro-store --features test-support`
- `cargo nextest run --workspace` — 7,656 passed, 203 skipped