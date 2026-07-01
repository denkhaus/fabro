Goal: # Plan A — `SecretRedactor` in `fabro-redact`

**This is Plan A of three** (split for parallel execution):

- **Plan A (this file)** — add a per-run secret-value redactor to `fabro-redact`.
  Self-contained; touches only `fabro-redact/`. **Run in parallel with Plan B.**
- **Plan B** — resolve `secrets.*` tokens at the run boundary. **Run in parallel
  with Plan A.**
- **Plan C** — wire redaction across leak surfaces + hooks. **Run after A and B
  merge** (it consumes this crate's type and Plan B's lookup).

This plan is inert on its own: it adds a tested library primitive that Plan C
wires up. Shipping it alone changes no behavior.

> **Token notation.** Interpolation tokens are written in this file without their
> enclosing double curly braces, so the file is safe to pass directly as a
> workflow goal (the goal templater would otherwise try to expand them). Read
> `secrets.NAME`, `env.NAME`, and `secrets.*` as the double-curly-brace-wrapped
> token form used everywhere else in the codebase, and write the real
> double-brace syntax in the code, tests, and docs you produce.

---

## Overall goal (shared context)

Make secret tokens (`secrets.NAME`) in workflow config resolve from the server
vault, at the run boundary, with values that never get persisted, never leak, and
fail closed when a secret is missing or the wrong type. The redaction guarantee
for declared secrets is: content-based redaction (already present) is the
universal baseline, plus a per-run registry of resolved secret **values** so a
declared secret is redacted even when it does not look like a credential. **This
plan builds that registry primitive.**

Why per-run and not a process global: a test-only in-process run path executes
multiple runs in the same process, so redaction state must be per-run, never a
`static`/global.

## Conventions

- **TDD.** Write the failing test first, then the code.
- Match the codebase: Rust import style (types by name, functions via parent
  module, no glob imports in production), `strum` for enum string maps, keep
  test-only helpers behind `#[cfg(test)]`.
- Plain-English commit messages, PR text, and comments — no internal planning
  identifiers.
- The verify gate runs nightly `fmt --check`, nightly
  `clippy --all-targets -D warnings`, `cargo nextest run --workspace`, docs check,
  web/api-client typecheck, and a release build. Implement so all pass.
- Never print or log a secret value.

---

## Implementation

### A.1 — Add the `SecretRedactor` type

File: new `lib/crates/fabro-redact/src/secret_registry.rs`, exported from
`lib/crates/fabro-redact/src/lib.rs`.

Add a cheap, cloneable, per-run registry of secret values that redacts exact
matches regardless of shape. It composes *after* the existing content-based
redaction (`redact_string`, `redact_json_value`) — this type does not replace
them.

Shape:

- `SecretRedactor` backed by shared, interior-mutable state (e.g.
  `Arc<Mutex<Vec<String>>>` or `Arc<RwLock<...>>`) so a clone handed to a
  different subsystem observes registrations. Derive `Clone` and `Default`; an
  empty redactor is a pure no-op.
- `fn register(&self, value: impl Into<String>)` — store a secret value to be
  redacted. **Ignore empty or whitespace-only values** (registering an empty
  string would turn all output into `REDACTED`). De-duplicate.
- `fn redact_into(&self, s: &str) -> String` — replace every registered value
  substring with the same `"REDACTED"` marker used by `redact_string`. Replace
  **longest values first** so a secret that is a substring of another is handled
  correctly. If the registry is empty, return the input unchanged (fast path).
- `fn redact_json(&self, value: serde_json::Value) -> serde_json::Value` — walk
  the JSON tree and apply `redact_into` to every string leaf (both object values
  and array elements; object keys are left as-is). Exact-value matching is
  unambiguous, so unlike `redact_json_value` this pass does not skip any keys.
- Optional `fn is_empty(&self) -> bool` for callers that want to skip work.

Reuse the crate's existing `"REDACTED"` replacement marker (see `redact_string`
in `lib.rs`) rather than introducing a new literal.

### A.2 — Tests (unit, in the new module)

- A **low-entropy** value (e.g. `"staging"`) that `redact_string` would *not*
  catch is replaced with `REDACTED` by `redact_into` after `register("staging")`.
- Registering `""` or `"   "` is a no-op: `redact_into` leaves unrelated text
  intact (guard against the empty-value footgun).
- Overlapping values: register both `"abc"` and `"abcdef"`; `redact_into` on a
  string containing `"abcdef"` redacts the whole token (longest-first), not just
  the `"abc"` prefix.
- Empty registry: `redact_into` and `redact_json` are the identity.
- `redact_json` redacts a registered value nested inside an object value and
  inside an array element.
- A clone of the redactor observes values registered through the original (shared
  state), proving it can be handed to another subsystem.

### A.3 — Verify

- `cargo +nightly-2026-04-14 fmt --check --all`
- `cargo +nightly-2026-04-14 clippy --workspace --all-targets -- -D warnings`
- `cargo nextest run -p fabro-redact`
- release build (`cargo dev build -- -p fabro-cli --release`)

## Dependencies

None. Parallel-safe with Plan B. This type is consumed by Plan C.


## Completed stages
- **toolchain**: succeeded
  - Script: `command -v cargo >/dev/null || { curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y && sudo ln -sf $HOME/.cargo/bin/* /usr/local/bin/; }; cargo --version 2>&1`
  - Output:
    ```
    cargo 1.95.0 (f2d3ce0bd 2026-03-21)
    ```
- **preflight_compile**: succeeded
  - Script: `cargo check -q --workspace 2>&1`
  - Output: (empty)
- **preflight_lint**: succeeded
  - Script: `cargo +nightly-2026-04-14 clippy -q --workspace --all-targets -- -D warnings 2>&1`
  - Output: (empty)


Read the plan file referenced in the goal and implement every step. Make all the code changes described in the plan. Use red/green TDD.