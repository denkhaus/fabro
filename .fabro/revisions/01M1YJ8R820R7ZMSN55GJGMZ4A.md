# Revision — run 01M1YJ8R820R7ZMSN55GJGMZ4A

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M1YJ8R820R7ZMSN55GJGMZ4A.md
- seeds filed: fabro-2254 (default-features clippy parity in implementer pre-gate check), fabro-c381 (minimal adjacent repair of pre-existing compile breaks in touched crates), fabro-8194 (carry implementer's prior diagnosis into gate-red bounce prompts), fabro-56db (normalize stale mtimes at qualitygate start), fabro-c841 (tighten gate-bounce known-bug matching to error signatures)
- basis: run 01M1YJ8R820R7ZMSN55GJGMZ4A, workflow version 4ff76f32665f954e9a8e2027ec2a4a5b48266707730ae0620fcfd3f79acd4a10, commit b1dbce3b713540dc605bec39d9ac09c17a224cb4
- revised_at_commit: b1dbce3b713540dc605bec39d9ac09c17a224cb4 (ADR-0015: engine drift signal for later judgement)

## Findings

### Mirror the gate's default-features clippy in the implementer's pre-gate check — filed fabro-2254
Require the literal gate invocation `cargo clippy -p <crate> --all-targets -- -D warnings` (default features) on every touched crate in `.fabro/workflows/develop/prompts/implementer.md` step 4. Expected effect: no more second cycles from feature-scoped-only verification (this run: ~8 min, ~$0.41, 64% of run cost wasted).

### Allow minimal adjacent repair of pre-existing compile breaks in touched crates — filed fabro-c381
Carve-out in `implementer.md` for minimal, disclosed repair of verified pre-existing breaks in touched crates. Expected effect: the 3-line fix gets applied instead of bouncing the whole run through a painpoint→gate-red loop.

### Carry the implementer's prior diagnosis into gate-red bounce prompts — filed fabro-8194
Add `implementation_summary` to `preamble_allow_keys` (or inject a `gate_bounce_notes` key) on the implementer node bounce path in `workflow.fabro`. Expected effect: implementer@2 stops re-deriving implementer@1's root cause (305s inference, $0.405, 73% of reasoning spend).

### Normalize stale mtimes at qualitygate start until the transport fix ships — filed fabro-56db
One-liner `find lib -name '*.rs' ! -newermt 2000-01-01 -exec touch {} +` at `scripts/qualitygate.nu` start, mirrored in `implementer.md` step 4, until fabro-22e4 ships. Expected effect: no stale-warning replay from 1970-epoch mtimes; zero LLM cost.

### Tighten gate-bounce known-bug matching to error signatures — filed fabro-c841
Require token overlap with error code/crate/file path in `.fabro/workflows/develop/scripts/gate-bounce.nu` instead of label matching; emit `{"hits":[]}` when nothing matches. Expected effect: E0432 failures stop pulling in ~1.4 KB of unrelated known-bug noise (false match on fabro-4814).
