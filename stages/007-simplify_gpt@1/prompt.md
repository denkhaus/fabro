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
- **implement**: succeeded
  - Model: gpt-5.5, 463.9k tokens in / 12.5k out
- **simplify_opus**: succeeded
  - Model: claude-opus-4-8, 37.4k tokens in / 15.3k out
  - Files: /home/daytona/workspace/fabro/lib/crates/fabro-redact/src/lib.rs, /home/daytona/workspace/fabro/lib/crates/fabro-redact/src/secret_registry.rs


# Simplify: Code Review and Cleanup

Review all changes for reuse, quality, and efficiency. Fix any issues found. Feel free to use any sub agents you need.

## Phase 1: Identify Changes

Run git diff (or git diff HEAD if there are staged changes) to see what changed. If there are no git changes, review the most recently modified files that the user mentioned or that you edited earlier in this conversation. (You may already have the changes in context, if so, feel free to skip this part)

## Phase 2: Launch Three Review Agents in Parallel

Use the Agent tool to launch all three agents concurrently in a single message. Pass each agent the full diff so it has the complete context.

### Agent 1: Code Reuse Review

For each change:

1. Search for existing utilities and helpers that could replace newly written code. Use Grep to find similar patterns elsewhere in the codebase — common locations are utility directories, shared modules, and files adjacent to the changed ones.
2. Flag any new function that duplicates existing functionality. Suggest the existing function to use instead.
3. Flag any inline logic that could use an existing utility — hand-rolled string manipulation, manual path handling, custom environment checks, ad-hoc type guards, and similar patterns are common candidates.

Note: This is a greenfield app, so focus on maximizing simplicity and don't worry about changing things to achieve it.

### Agent 2: Code Quality Review

Review the same changes for hacky patterns:

1. Redundant state: state that duplicates existing state, cached values that could be derived, observers/effects that could be direct calls
2. Parameter sprawl: adding new parameters to a function instead of generalizing or restructuring existing ones
3. Copy-paste with slight variation: near-duplicate code blocks that should be unified with a shared abstraction
4. Leaky abstractions: exposing internal details that should be encapsulated, or breaking existing abstraction boundaries
5. Stringly-typed code: using raw strings where constants, enums (string unions), or branded types already exist in the codebase

Note: This is a greenfield app, so be aggressive in optimizing quality.

### Agent 3: Efficiency Review

Review the same changes for efficiency:

1. Unnecessary work: redundant computations, repeated file reads, duplicate network/API calls, N+1 patterns
2. Missed concurrency: independent operations run sequentially when they could run in parallel
3. Hot-path bloat: new blocking work added to startup or per-request/per-render hot paths
4. Unnecessary existence checks: pre-checking file/resource existence before operating (TOCTOU anti-pattern) — operate directly and handle the error
5. Memory: unbounded data structures, missing cleanup, event listener leaks
6. Overly broad operations: reading entire files when only a portion is needed, loading all items when filtering for one

## Phase 3: Fix Issues

Wait for all three agents to complete. Aggregate their findings and fix each issue directly. If a finding is a false positive or not worth addressing, note it and move on — do not argue with the finding, just skip it.

When done, briefly summarize what was fixed (or confirm the code was already clean).
