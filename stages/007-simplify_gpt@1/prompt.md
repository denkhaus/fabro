Goal: # PR 4 — Resolve hook interpolation at the run boundary (and enable secrets in hooks)

**Self-contained implementation plan.** Everything needed to implement this is
in this file plus the repository. Independent — no preconditions; can land
anytime.

**Redaction context (fixed; do not build on it):** run-output redaction in
this codebase is **content-based only** — entropy + credential-pattern
detection (`fabro_redact::redact_string` / `redact_json_value`), applied
where events are serialized and where exec-output tails are captured. There
is no per-run exact-value secret registry (a registration approach was
considered and rejected). Secrets resolved for hooks by this PR get exactly
the coverage every other boundary-resolved secret (MCP env, run env, prepare
steps) already has: credential-shaped values are redacted from event and
tail surfaces if echoed; a low-entropy secret value is not — an accepted,
documented trade. Do not add any registration or exact-match machinery.

> **Token notation.** Interpolation tokens are written in this file without
> their enclosing double curly braces, so the file is safe to pass directly as
> a workflow goal (the goal templater would otherwise try to expand them).
> Read `secrets.NAME`, `env.NAME` as the double-curly-brace token form used in
> the codebase, and write the real double-brace syntax in the code, tests, and
> docs you produce.

## Context and goal

Hooks are user-defined callbacks on workflow lifecycle events (`run_start`,
`stage_start`, `pre_tool_use`, `sandbox_ready`, …) that can observe or gate a
run (decisions: proceed / block / skip / override). Four types: **command**
(shell, runs in the sandbox by default or host-side with `sandbox = false`),
**http** (POST from the worker), **prompt** and **agent** (LLM evaluation in
the worker). Their configurable string fields — `command`, `url`, header
values, `prompt`, `model` — are typed `InterpString` and may carry `env.NAME`
tokens.

Every other secret/env-consuming subsystem (MCP transport env, run-environment
env, prepare steps, docker config) resolves its InterpStrings **once, at the
run boundary** in `RunSession::new`, through one shared lookup closure. Hooks
are the single exception: the hook **executor** resolves tokens **at fire
time**, and only the `env` namespace is wired there — a `secrets.NAME` token
in a hook currently fails closed with "unavailable namespace". This fire-time
resolution is a fossil, not a decision: it dates from the original hooks
implementation, before the boundary-resolution pattern existed, and was
carried forward unexamined. A previous attempt to add secrets support built a
parallel fire-time secrets-resolution and redaction-registration subsystem
inside the hooks crate to accommodate it; that PR was closed, and the accepted
direction is to remove the root special case instead.

**Goal:** resolve all hook InterpStrings once at the run boundary through the
shared closures, hand the hooks subsystem fully-resolved strings, and delete
the executor's resolution layer. Consequences, all intended:

- `secrets.NAME` becomes usable in hook `command`, `url`, and `prompt`/`model`
  — the user-facing capability — with the same content-based redaction
  coverage every other boundary-resolved secret already has (see the
  redaction context above).
- The invariant "all env/secrets resolution happens at the run boundary" holds
  with **zero exceptions**, so no future secrets/redaction work needs a hooks
  special case.
- The hooks crate never learns about vaults or secrets at all.

Explicitly out of scope / preserved:

- The fire-time **context** mechanism is untouched: hooks receive per-firing
  data (event, node id, tool name, …) out-of-band via the `FABRO_HOOK_CONTEXT`
  env var, not via interpolation. There is no context namespace in
  InterpString; nothing interpolates per-firing data today, so boundary
  resolution loses no capability that exists.
- Matcher semantics, blocking/decision merging, sandbox-vs-host dispatch,
  timeouts: unchanged.

## Verified current state (as of main `9daca83b3`, 2026-07-09 — re-verify before starting; line numbers are anchors, not gospel)

`lib/crates/fabro-hooks/src/executor.rs`:

- `resolve_interp(value, env)` (~`:76`) — resolves an `InterpString` against
  process env at fire time; doc comment says only `env` is wired and other
  namespaces fail closed. Used for `command` and `url`.
- `resolve_prompt_and_model` (~`:186`) — same, for prompt/agent hooks.
- `resolve_header(value, allowed_env_vars, env)` (~`:132`) — header
  values additionally gate `env.NAME` behind the hook's `allowed_env_vars`
  list (`HeaderResolveError::NotAllowed`); resolution errors block the hook.
- `safe_url_source_for_log` — logs the **unresolved** URL source (never the
  resolved URL) so env-sourced URL material is not logged.
- Fail-closed dispositions today: a resolution error in `command` blocks; in
  `url`/headers/prompt/model the hook logs an error and does not fire (http
  headers produce a block); transport-level failures stay fail-open.

`lib/crates/fabro-types/src/settings/run.rs`:

- `HookDefinition { name, event, command: Option<InterpString>, hook_type,
  matcher, blocking, timeout_ms, sandbox }` (~`:2173`); `HookType::{Command,
  Http { url, headers, allowed_env_vars, tls }, Prompt { prompt, model },
  Agent { prompt, model } }` (~`:2149`). `vars.NAME` tokens in all these
  fields are already substituted at run creation (`substitute_variables`
  walks hooks), so only `env`/`secrets` tokens remain by boundary time.

`lib/crates/fabro-workflow/src/operations/start.rs`:

- The boundary pattern to mirror: `runtime_mcp_server(server, process_env_var,
  secret_lookup)` (~`:718`) and `runtime_setup_commands(...)` (~`:745`) —
  config type in, resolved runtime type out, hard error on missing names.
- Hooks are currently passed through to the runner **unresolved** (find the
  hook wiring where `resolved.hooks` reaches `HookSettings`).

Environment-timing note (verified): no `env::set_var` in production worker
paths, so worker process env is identical at boundary time and fire time —
resolving earlier does not change resolved values. Command hooks with
`sandbox = true` already resolve against **worker** env and ship the resolved
string into the sandbox; that stays true, just earlier.

## Design

1. **Runtime hook type.** Add a resolved runtime form (e.g.
   `RuntimeHookDefinition`, plain `String` fields, mirroring
   `HookDefinition`/`HookType` shape) plus a boundary constructor
   `runtime_hooks(hooks, process_env_var, secret_lookup) -> Result<Vec<...>>`
   in `operations/start.rs` alongside `runtime_mcp_server` /
   `runtime_setup_commands`. The config/wire type `HookDefinition` is
   unchanged — no API or manifest change. For http hooks, carry the
   **unresolved url source string** on the runtime type as well, for safe
   logging (preserves the `safe_url_source_for_log` guarantee).
2. **Header policy enforced at the boundary, unchanged in substance:**
   - a `secrets.NAME` token in a header value is rejected **before any vault
     lookup**, with the existing guidance shape: secrets are not allowed in
     HTTP hook headers; use secret interpolation in a hook command, prompt,
     or url instead;
   - `env.NAME` in header values stays gated by `allowed_env_vars`
     (non-allowlisted name → error naming the variable; allowlisted-but-unset
     → missing-variable error);
   - `command`/`url`/`prompt`/`model` resolve `env` + `secrets` with hard
     errors on missing names.
   Any resolution error **fails the run at startup** (consistent with how
   missing secrets in MCP/prepare config behave).
3. **Slim the executor.** `HookRunner`/`HookExecutor` take the runtime type;
   delete `resolve_interp`, `resolve_header`, `resolve_prompt_and_model`,
   `HeaderResolveError`, and the `Env` type parameters from execution paths.
   The executor formats, dispatches, and merges decisions — it resolves
   nothing.
4. **No hook-side redaction work needed.** Hook output and block/skip reasons
   flow into events, and event serialization already applies the content-based
   redaction pass (`event/redaction.rs`, `redact_json_value`). That is the
   full extent of coverage by design — do not add redaction machinery for
   hook values (see the redaction context at the top).

### Behavior changes (state these plainly in the PR description)

- **Fail timing moves earlier.** A hook referencing a missing env var or
  secret today fails when (and only if) the hook fires; after this PR the run
  fails at startup, including for hooks that would never have fired. Both are
  fail-closed; startup surfacing is stricter and reports config errors
  immediately instead of mid-run.
- **Eager resolution.** Hook secrets resolve even if the hook never fires;
  values are held in worker memory for the run, like every other
  boundary-resolved secret.
- Env snapshot timing is theoretically observable but a practical no-op (see
  the environment-timing note above).

## Implementation

1. Boundary: `RuntimeHookDefinition` + `runtime_hooks(...)` with header
   policy; wire into `RunSession::new` next to the other `runtime_*`
   resolvers; hard-fail the run on any resolve error.
2. `fabro-hooks`: switch `HookSettings`/runner/executor to the runtime type;
   delete the resolution layer; keep matcher/blocking/dispatch/
   `FABRO_HOOK_CONTEXT`/timeout code untouched.
3. Migrate tests:
   - executor tests asserting resolution behavior (missing env var blocks at
     fire time; header allowlist gating; unavailable-namespace errors) become
     boundary tests asserting startup failure / rejection with the same error
     content;
   - executor execution tests (dispatch, decisions, timeouts, sandbox-vs-host)
     switch to literal strings.
4. New end-to-end tests (worker level, hermetic temp-dir vaults):
   - command hook with a `secrets.NAME` token resolves from the vault and
     proceeds;
   - missing hook secret fails the run at startup, error names the secret;
   - `secrets.NAME` in an http-hook header fails at startup with the guidance
     message, and the endpoint is never called;
   - http hook with a secret-valued URL resolves and fires (mock server
     asserts the call);
   - a blocking command hook whose block reason echoes a resolved
     **credential-shaped** secret value (use a distinctive high-entropy test
     marker, never a realistic credential) has that value redacted in stored
     events by the existing content-based pass — proving hook secrets get the
     standard coverage. Do not assert redaction of low-entropy values; that
     is out of coverage by design;
   - a hook that references only env still works host-side and sandbox-side.
5. Docs (`docs/public/` hooks page): secrets usable in hook command / url /
   prompt; headers reject secret tokens with the guidance; missing names fail
   at run start.

## Scope boundaries — deliberately NOT in this PR

- **No redaction machinery inside `fabro-hooks`** — restated as a boundary:
  hook output flows into events, and event serialization already applies the
  content-based pass. If you find yourself adding a redactor, a registry, or
  a secrets type to the hooks crate, you have left this PR's design.
- **The event-serialization redaction pass in `event/redaction.rs`** — leave
  as-is; do not extend, scope, or restructure it for hook fields.
- **Exec-output tails and any `fabro-sandbox` redaction signatures** — leave
  as-is; they already apply content-based redaction.
- **Read-side server handlers and the event-detail `redacted` flag** — leave
  as-is; a separate change owns read paths.
- **Typed wrapper types for secret values** — separate planned work. The
  runtime hook type carries plain resolved `String`s in this PR.

If work outside these boundaries seems genuinely required for this PR to
compile or pass its tests, stop and state that in the PR description rather
than expanding scope.

## Acceptance / verification

- `cargo +nightly-2026-04-14 fmt --check --all`
- `cargo +nightly-2026-04-14 clippy --workspace --all-targets -- -D warnings`
- `cargo nextest run --workspace` (touched: `fabro-hooks`, `fabro-workflow`,
  `fabro-types` if the runtime type lands there)
- `cargo dev docs check`
- No OpenAPI/wire change (config types untouched).

## Conventions

- Never print or log a resolved secret value, including from tests; preserve
  unresolved-source-only URL logging.
- Plain-English commit messages, PR text, and comments; no internal planning
  identifiers or plan-file names in anything that ships.
- PR description must include the two behavior changes above, framed as
  intended semantics (fail-fast config errors), and state the capability
  added (vault secrets in hooks) with the header exclusion.
- If implementation uncovers a genuine need for per-firing interpolation in
  hook strings (none is known), stop and surface it rather than re-adding a
  fire-time resolver.


## Completed stages
- **toolchain**: succeeded
  - Script: `command -v cargo >/dev/null || { curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y && sudo ln -sf $HOME/.cargo/bin/* /usr/local/bin/; }; cargo --version 2>&1`
  - Output:
    ```
    cargo 1.96.0 (30a34c682 2026-05-25)
    ```
- **preflight_compile**: succeeded
  - Script: `cargo check -q --workspace 2>&1`
  - Output: (empty)
- **preflight_lint**: succeeded
  - Script: `cargo +nightly-2026-04-14 clippy -q --workspace --all-targets -- -D warnings 2>&1`
  - Output: (empty)
- **implement**: failed
- **simplify_fable**: succeeded
  - Model: claude-fable-5, 374.8k tokens in / 149.0k out
  - Files: /home/daytona/workspace/fabro/docs/public/agents/hooks.mdx, /home/daytona/workspace/fabro/docs/public/api-reference/fabro-api.yaml, /home/daytona/workspace/fabro/lib/crates/fabro-config/src/resolve/run.rs, /home/daytona/workspace/fabro/lib/crates/fabro-hooks/Cargo.toml, /home/daytona/workspace/fabro/lib/crates/fabro-hooks/src/bridge.rs, /home/daytona/workspace/fabro/lib/crates/fabro-hooks/src/config.rs, /home/daytona/workspace/fabro/lib/crates/fabro-hooks/src/executor.rs, /home/daytona/workspace/fabro/lib/crates/fabro-hooks/src/lib.rs, /home/daytona/workspace/fabro/lib/crates/fabro-hooks/src/runner.rs, /home/daytona/workspace/fabro/lib/crates/fabro-hooks/src/types.rs, /home/daytona/workspace/fabro/lib/crates/fabro-hooks/tests/host_command_hooks.rs, /home/daytona/workspace/fabro/lib/crates/fabro-types/src/settings/run.rs, /home/daytona/workspace/fabro/lib/crates/fabro-workflow/src/operations/start.rs, /home/daytona/workspace/fabro/lib/crates/fabro-workflow/src/pipeline/initialize.rs, /home/daytona/workspace/fabro/lib/crates/fabro-workflow/tests/it/integration.rs


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
