# Revision — run 01M2RDM3AZ5BRZXAPE4GT365P0

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2RDM3AZ5BRZXAPE4GT365P0.md
- seeds filed: fabro-533c — Render binary-classified diffs as text in evidence.nu; fabro-269d — Enforce reviewer read-only posture via per-node tool allow-list on the develop graph (needs-user)
- basis: run 01M2RDM3AZ5BRZXAPE4GT365P0, workflow version f4ab9ff48b6e162ee81f42689c29e27253fba4fe3632992c2e164afa31eb5ca5, commit 5b7e0356a7536b8fe457d78a6dd4660a70bd3ecb
- revised_at_commit: 5b7e0356a7536b8fe457d78a6dd4660a70bd3ecb (ADR-0015: engine drift signal for later judgement)

## Findings

### Render binary-classified diffs as text in evidence.nu
- filed: fabro-533c
- Change: in `.fabro/workflows/develop/scripts/evidence.nu`, fall back to `git diff --text` (or an escaped rendering) under existing size bounds when a diff classifies as binary. Expected effect: reviewer no longer burns 7/9 tool calls shell-reading a 93-line NUL-byte fixture; UNSEEN-item ambiguity that can escalate to Verification blocked is removed. No duplicate found (`sd search binary`, `sd search diff`).

### Enforce reviewer read-only posture via per-node tool allow-list on the develop graph
- filed: fabro-269d (capability-affecting, `needs-user,revision`, awaits explicit user approval per ADR-0019)
- Change: adopt closed fabro-47b5 per-node tool policy in `.fabro/workflows/develop/workflow.fabro` — reviewer gets read-only tools; implementer the same minus `spawn_agent`. Not a duplicate of fabro-47b5 (engine feature, closed) — this is the develop-graph adoption, cross-referenced in the seed. Expected effect: read-only contract becomes mechanical, capability-delta risk shrinks to diff-added tools.
