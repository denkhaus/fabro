Done. A note up front: the failed `implement` stage had produced **zero code changes** (API auth failure — all stage commits were empty), so I implemented PR 4 from scratch and then ran the three-agent review pass over my own diff.

## What was implemented

**Boundary resolution (the PR's core):**
- `fabro-types/src/settings/run.rs`: new `RuntimeHookDefinition`, `RuntimeHookType`, `RuntimeHttpHook`, and `HookResolveError`, plus `HookDefinition::resolve_env(env_lookup, secrets_lookup)` — resolves `command`/`url`/headers/`prompt`/`model` once, with the header policy enforced before any lookup (secrets in headers rejected with the guidance message; header env gated by `allowed_env_vars`). Http hooks carry the unresolved `url_source` for safe logging. Wire/config types are untouched.
- `fabro-workflow/operations/start.rs`: `runtime_hooks(...)` next to `runtime_mcp_server`/`runtime_setup_commands`, wired into `RunSession::new` through the shared `process_env_var`/`secret_lookup` closures; any resolution error fails the run at startup naming the hook.
- `fabro-hooks`: executor/runner/bridge now consume resolved strings only — deleted `resolve_interp`, `resolve_header`, `HeaderResolveError`, `resolve_prompt_and_model`, the `Env` type params, and the crate's `fabro-util` dep. Matchers, decision merging, `FABRO_HOOK_CONTEXT`, timeouts, and TLS enforcement are untouched. Also deleted the production-dead `HookSettings::merge` (config layering owns merge semantics).

**Tests:** resolution tests moved to fabro-types (13 new) and start.rs boundary tests (secret resolves / missing secret fails startup / header-secret guidance, incl. a panicking-lookup test proving rejection happens before any vault read); new worker-level tests with hermetic temp-dir vaults: command-hook secret proceeds, http hook with secret URL fires (httpmock asserts), blocking hook echoing a credential-shaped secret gets **redacted in stored events** by the existing content-based pass, and env-only hooks work on both host and sandbox dispatch.

**Docs:** `hooks.mdx` gained an Interpolation section (secrets in command/url/prompt, header exclusion, startup-fail semantics); the OpenAPI `headers` description was updated (description-only, no wire change).

## Review-agent findings fixed
- **Quality:** swappable `url`/`url_source` `&str` params → grouped into `RuntimeHttpHook`; stale fire-time wording in `fabro-config/resolve/run.rs` and `fabro-api.yaml`; missing allowlist-violation outcome in hooks.mdx; `Debug`-leak warning added to runtime type docs.
- **Reuse:** extracted shared `resolve_env_secrets` (now backing both `resolve_env_string` and hook resolution); test vault seeding reuses `fabro_auth::vault_set_token`.
- **Efficiency:** `initialize.rs` now **moves** the resolved hooks into `HookRunner` instead of deep-cloning them, so a single copy of resolved secrets lives per run. (Reviewers also confirmed the per-fire path got strictly lighter.)
- **Skipped (noted deliberately):** pre-redacting `url_source` at the boundary (plan mandates carrying the unresolved source; would add a fabro-redact dep to fabro-types), exporting `vault_token_lookup` just for a test one-liner, and rebuilding all test fixtures through `resolve_env`.

## Verification
`cargo +nightly-2026-04-14 fmt --check --all` ✓ · `clippy --workspace --all-targets -D warnings` ✓ · `cargo nextest run --workspace` → **6964 passed** ✓ · `cargo dev docs check` ✓

For the PR description, the two intended behavior changes: **fail timing moves to startup** (missing hook env/secret fails the run immediately, even for hooks that never fire) and **eager resolution** (hook secrets are resolved and held in worker memory for the run, like every other boundary-resolved secret).