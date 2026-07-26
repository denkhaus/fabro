---
title: Add CLI Variable Management
type: feat
status: active
date: 2026-05-27
---

# Add CLI Variable Management

## Overview

Expose the recently added variables API through the CLI with a singular `fabro variable`
namespace. Variables are non-sensitive run-configuration values, so the CLI should expose
values in `list` and `get`, while continuing to direct credentials and tokens to
`fabro secret`.

## Requirements Trace

- R1. Provide variables management in the CLI, similar to secrets management.
- R2. Support the full readable-variable CRUD surface: list, get, set/upsert, and remove.
- R3. Preserve existing server/API behavior: variable names are env-style, values may be
  empty, and `set` preserves an existing description when `--description` is omitted.
- R4. Keep generated CLI docs and help snapshots in sync with the new public command.

## Context & Research

- `lib/crates/fabro-cli/src/commands/secret/` is the command pattern to follow for
  namespace dispatch, JSON output, tabular list output, stdin value input, and status
  messages.
- `lib/crates/fabro-server/src/server/handler/variables.rs` already provides
  `GET /variables`, `POST /variables`, `GET /variables/{name}`,
  `PUT /variables/{name}`, and `DELETE /variables/{name}`.
- `lib/crates/fabro-types/src/variable.rs` defines the canonical API/request types and
  validates env-style names.
- `lib/crates/fabro-api/tests/variable_round_trip.rs` already proves OpenAPI generated
  types reuse the canonical variable types.
- `docs/public/workflows/variables.mdx` currently explains workflow template variables
  but does not yet document how server-managed `{{ "{" ~ "{" }} "{" ~ "{" {{ "}" ~ "}" }} vars.NAME {{ "{" ~ "{" }} "}" ~ "}" {{ "}" ~ "}" }}` values are configured.

## Key Technical Decisions

- Use `fabro variable`, not `fabro variables`, to match existing singular CLI namespaces
  such as `fabro secret`, `fabro model`, and `fabro repo`.
- Add `get` because variables are intentionally readable; secrets remain write-only.
- Make `set` an upsert using the API's create/upsert endpoint, matching the mental model
  of `fabro secret set`.
- Reuse `--value-stdin` from secrets but allow empty stdin values for variables after
  trimming trailing newlines.
- Plain `list` should include a `VALUE` column. Do not add truncation or redaction in
  this first pass; exact retrieval is available through JSON output and `get`.

## Implementation Units

- [ ] **Unit 1: Add fabro-client variable wrappers**

**Goal:** Give CLI code stable methods over the generated OpenAPI client.

**Requirements:** R2, R3

**Dependencies:** Existing variables API and generated `fabro-api` client.

**Files:**
- Modify: `lib/crates/fabro-client/src/client.rs`

**Approach:**
- Add wrappers for `list_variables`, `get_variable`, `create_variable`,
  `update_variable`, and `delete_variable`.
- Return `Vec<types::Variable>` from `list_variables` by unwrapping the API response's
  `data`, matching `list_secrets`.
- Use the generated path-parameter operations for `get`, `update`, and `delete`.

**Patterns to follow:**
- `list_secrets`, `create_secret`, and `delete_secret_by_name` in the same file.

**Test scenarios:**
- Happy path: CLI integration tests in later units exercise each wrapper through the
  shared server client path.
- Error path: missing and invalid variable operations propagate the server's API errors.

**Verification:**
- The CLI can compile against these wrapper methods without importing generated client
  builders directly.

- [ ] **Unit 2: Add CLI args, dispatch, and command module**

**Goal:** Register the new top-level namespace and route subcommands to implementation
modules.

**Requirements:** R1, R2, R4

**Dependencies:** Unit 1

**Files:**
- Modify: `lib/crates/fabro-cli/src/args.rs`
- Modify: `lib/crates/fabro-cli/src/main.rs`
- Modify: `lib/crates/fabro-cli/src/commands/mod.rs`
- Create: `lib/crates/fabro-cli/src/commands/variable/mod.rs`

**Approach:**
- Add `Commands::Variable(VariableNamespace)` with description
  `Manage server-owned variables`.
- Add `VariableNamespace` with `ServerTargetArgs`, matching `SecretNamespace`.
- Add `VariableCommand::{List, Get, Rm, Set}`; give `list` the `ls` alias.
- Add command-name mapping for analytics/logging: `variable list`, `variable get`,
  `variable rm`, and `variable set`.
- Dispatch through `commands::variable::dispatch`, deriving the target context with
  `base_ctx.with_target(&ns.target)`.

**Patterns to follow:**
- `SecretNamespace`, `SecretCommand`, and `commands::secret::dispatch`.

**Test scenarios:**
- Happy path: `fabro --help` lists `variable`.
- Happy path: `fabro variable --help` shows `list`, `get`, `rm`, and `set`.
- Happy path: command-name mapping covers all subcommands.

**Verification:**
- The new namespace is reachable through clap and main dispatch without affecting
  existing commands.

- [ ] **Unit 3: Implement variable list/get/set/rm behavior**

**Goal:** Provide the full user-facing variables management workflow.

**Requirements:** R1, R2, R3

**Dependencies:** Units 1 and 2

**Files:**
- Create: `lib/crates/fabro-cli/src/commands/variable/list.rs`
- Create: `lib/crates/fabro-cli/src/commands/variable/get.rs`
- Create: `lib/crates/fabro-cli/src/commands/variable/set.rs`
- Create: `lib/crates/fabro-cli/src/commands/variable/rm.rs`

**Approach:**
- `list`: fetch all variables, print JSON array when JSON output is active, otherwise
  print a table with `NAME`, `VALUE`, and `UPDATED`.
- `get`: fetch one variable, print the full variable object for JSON output, otherwise
  print only the raw value to stdout.
- `set`: accept `<NAME> [VALUE]`, `--value-stdin`, and `--description`; call the upsert
  API wrapper and print the stored variable for JSON output or `Set NAME` otherwise.
- `rm`: call the delete API wrapper and print `{ "name": NAME }` for JSON output or
  `Removed NAME` otherwise.
- For `set`, allow empty explicit values and empty stdin values. Only error when no value
  is provided and stdin is not being used.

**Patterns to follow:**
- `commands/secret/list.rs` for table style and age formatting.
- `commands/secret/set.rs` for argument precedence and stdin handling, adjusted so empty
  values are valid.
- `commands/secret/rm.rs` for delete output shape.

**Test scenarios:**
- Happy path: `set DEPLOY_ENV staging --description "Deployment target"` then `list`
  shows `DEPLOY_ENV`, `staging`, and an updated age.
- Happy path: `get DEPLOY_ENV` prints exactly `staging\n` in plain output.
- Happy path: `set DEPLOY_ENV production` updates the value and preserves the existing
  description through API behavior.
- Happy path: `set EMPTY ""` stores an empty value.
- Happy path: `printf '\n' | fabro variable set EMPTY --value-stdin` stores an empty
  value instead of failing.
- Error path: `get MISSING` and `rm MISSING` fail with `variable not found: MISSING`.
- Error path: `set 1BAD value` fails with the server invalid-name error.

**Verification:**
- The command works against the default test server and does not write directly to
  local `variables.json`.

- [ ] **Unit 4: Add test harness support and CLI integration tests**

**Goal:** Lock the public CLI surface and expected behavior with integration coverage.

**Requirements:** R1, R2, R3, R4

**Dependencies:** Units 1-3

**Files:**
- Modify: `lib/crates/fabro-test/src/lib.rs`
- Modify: `lib/crates/fabro-cli/tests/it/cmd/mod.rs`
- Modify: `lib/crates/fabro-cli/tests/it/cmd/fabro.rs`
- Modify: `lib/crates/fabro-cli/tests/it/cmd/json_global.rs`
- Create: `lib/crates/fabro-cli/tests/it/cmd/variable.rs`
- Create: `lib/crates/fabro-cli/tests/it/cmd/variable_list.rs`
- Create: `lib/crates/fabro-cli/tests/it/cmd/variable_get.rs`
- Create: `lib/crates/fabro-cli/tests/it/cmd/variable_set.rs`
- Create: `lib/crates/fabro-cli/tests/it/cmd/variable_rm.rs`

**Approach:**
- Add `TestContext::variable()` helper mirroring `TestContext::secret()`.
- Add help snapshots for the namespace and each subcommand.
- Add lifecycle tests for set/list/get/update/rm, `ls` alias, empty value support, JSON
  output, missing variable errors, and invalid-name errors.
- Update root help and curated landing snapshots only if the final clap/landing output
  changes.

**Patterns to follow:**
- `secret.rs`, `secret_list.rs`, `secret_set.rs`, and `secret_rm.rs`.

**Test scenarios:**
- Happy path: JSON `list` returns an array of full variable objects including `value`.
- Happy path: JSON `get` and `set` return full variable objects.
- Happy path: global JSON config makes `variable list` emit JSON, matching the
  `secret list` config test.
- Error path: missing variables and invalid names produce nonzero exits and readable
  errors.

**Verification:**
- `cargo nextest run -p fabro-cli -- variable`
- `cargo nextest run -p fabro-cli -- fabro`

- [ ] **Unit 5: Update generated and conceptual docs**

**Goal:** Keep public documentation aligned with the new command and clarify how variables
relate to secrets.

**Requirements:** R1, R4

**Dependencies:** Units 2-4

**Files:**
- Modify: `docs/public/reference/cli.mdx`
- Modify: `docs/public/workflows/variables.mdx`

**Approach:**
- Regenerate the CLI reference with `cargo dev docs refresh`.
- Add a short section to `docs/public/workflows/variables.mdx` explaining that
  server-managed run config variables can be set with `fabro variable set NAME VALUE`
  and referenced as `{{ "{" ~ "{" }} "{" ~ "{" {{ "}" ~ "}" }} vars.NAME {{ "{" ~ "{" }} "}" ~ "}" {{ "}" ~ "}" }}` in run config interpolation.
- State that variables are non-sensitive and readable; tokens, keys, and credentials
  should use `fabro secret set`.

**Patterns to follow:**
- Existing generated docs workflow in `lib/crates/fabro-dev/src/commands/docs.rs`.
- Existing CLI references to `fabro secret set` in administration docs.

**Test scenarios:**
- Happy path: generated CLI docs include `fabro variable` and its subcommands.
- Documentation check: `cargo dev docs check` succeeds after regeneration.

**Verification:**
- The docs describe the CLI surface without implying variables are secret storage.

## System-Wide Impact

- **API surface parity:** No server or OpenAPI changes are planned; the CLI consumes the
  existing variables API.
- **Error propagation:** Invalid names, missing variables, and write failures should flow
  through the existing `fabro-client` API error classification.
- **State lifecycle risks:** CLI commands must use the server API rather than editing
  `variables.json` locally, so behavior remains correct for remote and socket-backed
  servers.
- **Security boundary:** Values are intentionally visible for variables. Documentation
  must clearly distinguish variables from secrets to avoid accidental credential storage.
- **Unchanged invariants:** `fabro secret` remains write-only and unchanged.

## Risks & Dependencies

| Risk | Mitigation |
| --- | --- |
| Users put credentials in variables because the command looks like secrets | Document variables as non-sensitive and keep secret guidance explicit. |
| Empty values accidentally fail because secret handling rejects empties | Test explicit empty strings and newline-only stdin for `variable set`. |
| CLI docs drift after adding clap args | Regenerate with `cargo dev docs refresh` and verify with `cargo dev docs check`. |
| Plain `list` becomes awkward for long values | Accept for v1; `get` and JSON output provide exact machine-readable retrieval. |

## Verification Plan

- `cargo nextest run -p fabro-cli -- variable`
- `cargo nextest run -p fabro-cli -- fabro`
- `cargo dev docs check`
- `cargo +nightly-2026-04-14 fmt --check --all`
- `cargo +nightly-2026-04-14 clippy --workspace --all-targets -- -D warnings`

## Assumptions

- The chosen CLI surface is full CRUD with readable values.
- The namespace is singular: `fabro variable`.
- No TypeScript client regeneration is required for this CLI-only change.
- No server API, OpenAPI schema, or storage migration changes are required.
