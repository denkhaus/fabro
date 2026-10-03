# Revision — run 01M2N4XEB1XQW4466CXB57YJ4K

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2N4XEB1XQW4466CXB57YJ4K.md
- seeds filed: fabro-b22e — Prune the dead verification-only section from the implementer prompt
- basis: run 01M2N4XEB1XQW4466CXB57YJ4K, workflow version 6ad783be021e539bec3ef20d5ab0773111d20471eab6b0892bc38e48239d1d19, commit a3a9e6afa01c241bedc74fcf75029ca3c8468017
- revised_at_commit: a3a9e6afa01c241bedc74fcf75029ca3c8468017 (ADR-0015: engine drift signal for later judgement)

## Findings

### Prune the dead verification-only section from the implementer prompt
- filed: fabro-b22e (priority 2, not a duplicate: fabro-578a targets verification-only brief command scope, not dead prompt text; fabro-9d26 is closed and created the fast path that made this section dead)
- Change: remove or reduce to a one-line pointer the `## Verification-only briefs` section in `.fabro/workflows/develop/prompts/implementer.md` (lines ~133-135). The run's own merge (PR #181) routes verification-only claims planner->evidence->reviewer, so the implementer never receives them.
- Expected effect: smaller implementer prompt; no stale instructions that would mislead if the routing ever regresses.
