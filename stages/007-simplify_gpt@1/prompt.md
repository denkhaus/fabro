Goal: # Demote `server.integrations.slack.default_channel` to a plain literal string

**Self-contained implementation plan.** Everything needed to implement this is
in this file plus the repository. Independent — no preconditions; can land
anytime. (It must land before a separate, later effort freezes a registry of
config-field kinds, but nothing in this plan depends on that.)

> **Token notation.** Interpolation tokens are written in this file without
> their enclosing double curly braces, so the file is safe to pass directly as
> a workflow goal (the goal templater would otherwise try to expand them).
> Read `env.NAME`, `vars.NAME`, `secrets.NAME` as the double-curly-brace token
> form used in the codebase, and write the real double-brace syntax in the
> code, tests, and docs you produce.

## Context and goal

`server.integrations.slack.default_channel` is the last server-defined config
field still typed `InterpString`. Every other server-scope field was demoted
to a plain literal under the project's rule that interpolation belongs to
fields resolved with run context — server-startup consumption is served
natively by shells/compose/systemd, and a token there just ferries an env var
across a process boundary.

This field is exactly that case:

- It was **born `Option<String>`** (2026-03-05, in the original Slack
  integration crate; adopted into server settings 2026-04-07) and was
  documented from the start with literal examples only. The v2 settings
  schema typed it `InterpString` (2026-04-09) and wired env resolution the
  same day as part of that schema's uniform staged design — not a
  field-specific feature, and never requested or documented as interpolable.
  The one place env-interpolated Slack channels ARE a deliberate, documented
  feature is run-scope notification routes (added 2026-05-23) — a surface
  this change does not touch. The uniform-staging capability was superseded
  by the later interpolation-taxonomy decision that startup-consumed server
  fields stay literal, under which every comparable server field was already
  demoted.
- It resolves **once, at server startup, env-only** — no secrets, no vars,
  and never re-resolved at message-send time.
- **No documentation ever advertised interpolation** on it; every doc example
  is a plain literal channel name.
- The **run-scope interpolating surface already exists** and is the
  user-facing one: `run.notifications.<route>.slack.channel` and
  `run.interviews.slack.channel` are `InterpString` with variable
  substitution at run creation. The server field is only the zero-config
  fallback destination for interview prompts.
- Product direction reinforces it: in hosted deployments users have no access
  to server env at all (their surface is variables and secrets at run scope),
  and a future chat-integration plugin system should inherit a simple literal
  field, not a special-case interpolating one.

**Goal:** change the field to a plain `Option<String>` end-to-end, drop the
startup resolution, and emit the standard demoted-field warning when a
leftover token-shaped value is found — matching how the earlier server-field
demotions were shipped.

Design rules (fixed):

- **Keep the field** as the operator's literal fallback. Do not remove it or
  relocate it; run scope already covers per-run needs.
- **Wire-invisible.** `InterpString` serializes as its raw source string, so
  the stored/wire JSON shape is unchanged by this demote. The OpenAPI schema
  already models the field as a plain string — no spec change.
- **Warn, don't break.** A value still containing a token-shaped span (double
  curly braces) parses fine as a literal; emit the existing demoted-field
  warning at resolve time so the ~3 months of nightly builds where an env
  token would have resolved get a loud, non-fatal migration signal.

## Verified current state (as of main `d5dcd1179`, 2026-07-09 — re-verify before starting; line numbers are anchors, not gospel)

- Type: `lib/crates/fabro-types/src/settings/server.rs:258-271` —
  `SlackIntegrationSettings { enabled: bool, default_channel:
  Option<InterpString> }`.
- Config layer: `lib/crates/fabro-config/src/layers/server.rs:237` —
  `default_channel: Option<InterpString>`.
- Resolve copy-through: `lib/crates/fabro-config/src/resolve/server.rs:365-369`
  (clones the field into resolved server settings).
- Startup resolution: `lib/crates/fabro-server/src/server.rs:2422-2430` —
  `slack_settings.default_channel.as_ref().map(|value|
  value.resolve(process_env_var)...)` feeding
  `SlackService::new(bot, app, default_channel: Option<String>)`
  (`server.rs:589-600`). The service posts interview prompts to it
  (`server.rs:~657` `let Some(default_channel) = ... else return`, `:689`
  `post_message`).
- Demoted-field warning helper: `warn_if_demoted_template` in
  `lib/crates/fabro-config/src/resolve/` (see its callers in
  `resolve/cli.rs:50-76` for the exact usage pattern: field path string +
  `Option<&str>` value).
- Run-scope channels (untouched by this PR):
  `run.notifications.<route>` slack channel and
  `run.interviews.slack.channel`, both `InterpString`, variable-substituted at
  run creation (`fabro-types/src/settings/run.rs`, `substitute_variables`).
- Docs mentioning the field (all literal examples):
  `docs/public/administration/server-configuration.mdx:453`,
  `docs/public/human-tools/interviews.mdx:97`,
  `docs/public/integrations/slack.mdx:112-117`.
- OpenAPI: `docs/public/api-reference/fabro-api.yaml:13619-13623` models
  `default_channel` as a nullable plain string in the relevant schema —
  expected to need **no change**.

## Implementation

1. **Type change**: `fabro-types/src/settings/server.rs` and
   `fabro-config/src/layers/server.rs` — `Option<InterpString>` →
   `Option<String>`. Chase the compiler through the resolve copy-through and
   any settings merge/serde helpers.
2. **Demotion warning**: at the server-settings resolve site, call the
   existing demoted-field warning helper with the field path
   `server.integrations.slack.default_channel` and the literal value,
   following the exact pattern of its existing callers. The warning must log
   the field path and guidance only — never treat the value as sensitive
   output beyond what the existing helper does.
3. **Drop the startup resolve**: `fabro-server/src/server.rs` — pass the
   literal through to `SlackService::new` directly; delete the
   `resolve(process_env_var)` call and its error mapping. `SlackService`
   itself is unchanged (it already takes `Option<String>`).
4. **Verify (read-only) the interview routing preference**: confirm whether
   the interview-prompt posting path prefers `run.interviews.slack.channel`
   over the server default when both are set. If it does not, **do not build
   routing changes** — record the finding in the PR description as a
   follow-up observation.
5. **Docs**: the three pages above already show literals; adjust wording only
   if any implies interpolation (none is expected to). If the generated
   options reference annotates the field's type, regenerate via
   `cargo dev docs` and confirm `cargo dev docs check` is green.

## Scope boundaries — deliberately NOT in this PR

- **Run-scope notification and interview channel fields** — leave as-is; they
  are the intended interpolating surface and are already correct.
- **Slack credential resolution** (bot/app tokens via the vault at startup) —
  leave as-is; unrelated to the channel field.
- **Interview prompt routing behavior** — observe and report only (step 4);
  changing which channel wins is separate product work.
- **Any chat-integration plugin restructuring** — future work; this PR only
  simplifies what that redesign will inherit.
- **The config-field kind registry and its conformance tests** — separate
  planned work; do not start it here.

If work outside these boundaries seems genuinely required for this PR to
compile or pass its tests, stop and state that in the PR description rather
than expanding scope.

## Tests (failing-first; hermetic — no ambient env dependence)

- A literal channel (`#releases`) parses, merges, and reaches
  `SlackService::new` unchanged.
- A value containing a token-shaped span parses as a **literal** (no
  resolution, no error) and emits the demoted-field warning naming
  `server.integrations.slack.default_channel` (log-capture, matching how the
  existing demotion-warning tests assert).
- Startup wiring: with the field set, the service receives exactly the
  configured string; with it absent, `None` (existing behavior preserved).
- Existing Slack/server integration tests pass unchanged.
- Serde shape: a settings round-trip of the field produces the same JSON/TOML
  as before the type change (wire-invisibility).

## Acceptance / verification

- `cargo +nightly-2026-04-14 fmt --check --all`
- `cargo +nightly-2026-04-14 clippy --workspace --all-targets -- -D warnings`
- `cargo nextest run --workspace`
- `cargo dev docs check`
- No OpenAPI/wire change (assert: `docs/public/api-reference/fabro-api.yaml`
  untouched; TypeScript client regeneration not needed).

## Conventions

- Plain-English commit messages, PR text, and comments — describe what the
  change does; no internal planning identifiers or plan-file names in
  anything that ships.
- PR description must state plainly: the field only ever resolved env vars,
  only at server startup, and was never documented as interpolable; anyone
  with a token in the field gets a startup warning and the value is treated
  as a literal; per-run channels (`run.notifications`, `run.interviews.slack`)
  remain the interpolating surface; no wire change.


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
  - Model: claude-fable-5, 86.4k tokens in / 31.3k out
  - Files: /home/daytona/workspace/fabro/lib/crates/fabro-config/src/layers/server.rs, /home/daytona/workspace/fabro/lib/crates/fabro-config/src/resolve/server.rs, /home/daytona/workspace/fabro/lib/crates/fabro-config/src/tests/resolve_server.rs, /home/daytona/workspace/fabro/lib/crates/fabro-server/src/server.rs, /home/daytona/workspace/fabro/lib/crates/fabro-server/src/server/handler/system.rs, /home/daytona/workspace/fabro/lib/crates/fabro-server/src/server/tests.rs, /home/daytona/workspace/fabro/lib/crates/fabro-types/src/settings/server.rs


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
