Goal: # Remove the unused per-run secret registry (`SecretRedactor`) and stale references to it

**Self-contained implementation plan.** Everything needed to implement this is
in this file plus the repository. Independent — no preconditions; can land
anytime.

> **Token notation.** Interpolation tokens are written in this file without
> their enclosing double curly braces, so the file is safe to pass directly as
> a workflow goal (the goal templater would otherwise try to expand them).
> Read `env.NAME`, `secrets.NAME` as the double-curly-brace token form used in
> the codebase.

## Context and goal

Fabro redacts secrets from run output using **content-based** detection:
entropy analysis plus gitleaks-style credential patterns
(`fabro_redact::redact_string` / `redact_json_value`), applied where events are
serialized and where exec-output tails are captured.

A second mechanism was staged but never adopted: `SecretRedactor`, a per-run
registry of exact secret values, intended to be populated when declared
secrets resolve at the run boundary and then substituted out of run output
(catching low-entropy secret values that content-based detection cannot). The
type landed as infrastructure ahead of its wiring; the wiring PR was
ultimately **not merged** — the team decided the registration approach was too
much plumbing for too little benefit over the existing content-based
redaction, and content-based redaction is now the settled mechanism.

That leaves dead code and two stale forward references on main:

- `SecretRedactor` has **zero consumers** outside its own crate — nothing
  constructs, registers into, or applies it anywhere in the workspace.
- A doc comment in `fabro-auth` says provider-header secret resolution sits
  outside the registry "until exact-match registration is threaded through" —
  a follow-up that will never happen.
- The `InterpString` module doc in `fabro-types` says declared-secret values
  "are intended to be registered into a per-run exact-value redactor" —
  describing the abandoned design as if it were pending.

**Goal:** delete the dead type and rewrite both stale comments so the code
describes the real architecture (content-based redaction only). Pure
deletion/documentation PR — no behavior change.

## Verified current state (as of main `9daca83b3`, 2026-07-09 — re-verify before starting; line numbers are anchors, not gospel)

- `lib/crates/fabro-redact/src/secret_registry.rs` — the whole module
  (~217 lines: `SecretRedactor` with `register`, `is_empty`, redaction
  methods, and its unit tests). Uses `crate::Region`, which is **shared** with
  `entropy.rs` and `gitleaks.rs` and must stay.
- `lib/crates/fabro-redact/src/lib.rs:11` — `mod secret_registry;` and `:15`
  `pub use secret_registry::SecretRedactor;`.
- Workspace-wide grep for `SecretRedactor` outside `fabro-redact` returns
  nothing (no consumers in `lib/`, `apps/`, or `docs/`). If this grep finds a
  consumer when you run it, **stop** — the premise of this plan no longer
  holds; state that instead of deleting.
- `lib/crates/fabro-auth/src/resolve.rs:479-482` — doc comment on
  `resolve_extra_headers`:
  "Provider header secrets resolve outside the run-boundary redactor
  registration path. Keep this path free of value logging until exact-match
  registration is threaded through."
- `lib/crates/fabro-types/src/settings/interp.rs:17-19` — module doc sentence:
  "Declared-secret values are intended to be registered into a per-run
  exact-value redactor where secrets resolve; sensitivity is not tracked on
  resolved strings."

## Implementation

1. **Delete the module**: remove
   `lib/crates/fabro-redact/src/secret_registry.rs`, the `mod secret_registry;`
   declaration, and the `pub use secret_registry::SecretRedactor;` re-export
   from `lib.rs`. Leave `Region`, `redact_string`, `redact_json_value`,
   `DisplaySafeUrl`, and everything else in the crate untouched.
2. **Rewrite the `fabro-auth` comment** on `resolve_extra_headers`: keep the
   operative guidance (never log resolved header values — they may contain
   secrets), drop the promise of future exact-match registration. Suggested
   shape: "Resolved header values may contain secrets; keep this path free of
   value logging. Content-based redaction covers credential-shaped values on
   output surfaces, but nothing substitutes these exact values."
3. **Rewrite the `interp.rs` module-doc sentence**: state the real
   architecture — resolved secret values are plain strings; sensitivity is not
   tracked on resolved strings; redaction of run output is content-based
   (entropy + credential patterns), applied where output is serialized. Do not
   reference a registry or any pending mechanism.
4. **Sweep for stragglers**: `rg -n "SecretRedactor|secret_registry|exact-match|exact-value" lib/ docs/internal/`
   — any remaining hit that describes per-run exact-value redaction as
   existing or planned must be removed or rewritten in this PR. (Expected
   after steps 1–3: no hits.)

## Scope boundaries — deliberately NOT in this PR

- **Content-based redaction** (`redact_string`, `redact_json_value`, the
  entropy/gitleaks finders, `Region`) — untouched. This PR removes the unused
  second mechanism, not the working first one.
- **Where content-based redaction is applied** (event serialization,
  exec-output tails, server read paths) — no changes to any application site;
  this PR does not move, add, or remove redaction passes.
- **`DisplaySafeUrl` and redacting `Debug` impls** — untouched; unrelated
  pattern.
- **The live command-output log path** — has no redaction today; a separate
  planned change addresses it. Do not touch it here.
- **`fabro-hooks`** — untouched.

If work outside these boundaries seems genuinely required for this PR to
compile or pass its tests, stop and state that in the PR description rather
than expanding scope.

## Tests

- No new tests: the deleted module's tests go with it; no behavior changes to
  test. Existing `fabro-redact` tests (entropy, gitleaks, jsonl, safe-url)
  must pass unchanged.
- `cargo build --workspace` proves no hidden consumer existed.

## Acceptance / verification

- `cargo +nightly-2026-04-14 fmt --check --all`
- `cargo +nightly-2026-04-14 clippy --workspace --all-targets -- -D warnings`
- `cargo nextest run --workspace`
- The sweep in step 4 returns clean.
- No OpenAPI/wire change; no TypeScript client regeneration.

## Conventions

- Plain-English commit messages, PR text, and comments — describe what the
  change does; no internal planning identifiers or plan-file names in
  anything that ships.
- PR description must state plainly: the exact-value registry approach was
  abandoned in favor of the existing content-based redaction; the type was
  never wired to any consumer; the two rewritten comments previously promised
  the abandoned mechanism. Known limitation to state honestly: low-entropy
  declared secret values (e.g. a secret whose value is an ordinary word) are
  not caught by content-based detection — this is an accepted trade, not a
  regression introduced here.


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
- **implement**: failed

## Context
- failure_class: deterministic
- failure_signature: implement|deterministic|api_deterministic|openai|authentication


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
