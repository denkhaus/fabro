## Summary

The prior "implement" stage had failed on an API authentication error with **zero changes committed**, so I implemented the plan from scratch, then ran the three-agent simplify review and applied its findings.

### Implementation
- **Type demotion**: `default_channel: Option<InterpString>` → `Option<String>` in `fabro-types/src/settings/server.rs:261` and `fabro-config/src/layers/server.rs:237` (unused `InterpString` import removed from fabro-types).
- **Demotion warning**: `resolve_integrations` in `fabro-config/src/resolve/server.rs` now calls `warn_if_demoted_template("server.integrations.slack.default_channel", ...)`, mirroring the adjacent GitHub arm.
- **Dropped startup resolution**: `fabro-server/src/server.rs:2425` passes the literal straight to `SlackService::new`; the `resolve(process_env_var)` call and its error mapping are gone. Deleted the now-single-caller `display_interp` helper in `server/handler/system.rs` — the `/system/integrations` endpoint no longer does per-request template resolution.
- **Tests**: literal pass-through + JSON wire-shape lock, token-stays-literal (mirroring `resolve_storage_root_keeps_template_token_literal`), and startup wiring tests (set → verbatim `#releases`; absent → `None`).

### Step-4 finding (read-only, for the PR description)
The interview-prompt posting path (`SlackService::handle_event`, server.rs:657) posts **only** to the server `default_channel` and never consults `run.interviews.slack.channel` — that run-scope field is parsed and resolved but has no consumer in the posting path. Follow-up product work, not changed here. Also worth noting in the PR: previously an unresolvable env token **failed startup**; now it flows through verbatim with a resolve-time warning — the intended "warn, don't break" behavior.

### Simplify review results
- **Fixed**: extracted `slack_test_vault_tokens()` to replace three verbatim copies of the vault-token fixture; aligned `system.rs` to the sibling `as_ref()` + clone-at-insert pattern.
- **Skipped (with reason)**: dropping the JSON wire-shape assert (plan explicitly requires the serde wire-invisibility test) and folding the two wiring tests into one (plan requires both the set and absent cases).
- Reviewers confirmed clean: no dead code, correct helper reuse, field-path string style matches siblings, private-field asserts match module test style, and the diff is a net perf win (startup and per-request resolution both removed).

### Verification
- `cargo +nightly-2026-04-14 fmt --check --all` — clean
- `cargo +nightly-2026-04-14 clippy --workspace --all-targets -- -D warnings` — clean
- `cargo nextest run --workspace` — 6961 passed (before final test-file cleanup; fabro-server/fabro-config re-run after: 984 passed)
- `cargo dev docs check` — up to date; the three doc pages already show literals only
- `docs/public/api-reference/fabro-api.yaml` — untouched; no wire change, no TS client regen needed