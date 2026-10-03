# Revision — run 01M2K73FA670FSBBYN0WJV0YTE

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2K73FA670FSBBYN0WJV0YTE.md
- seeds filed: fabro-b09c — Carry tool-result errors through the provider protocol instead of stdout text-sniffing; fabro-4bb7 — Backfill pull_request.state for terminal-but-unmerged runs in the fabro_runs_list projection
- basis: run 01M2K73FA670FSBBYN0WJV0YTE, workflow version ab2ee24a456a34dfd349075bbbf8a38de2d071f30582720402cc4fedd93a2170, commit 62efab0764b33a87b794406141628d4b874e5d03
- revised_at_commit: 62efab0764b33a87b794406141628d4b874e5d03 (ADR-0015: engine drift signal for later judgement)

## Findings

### Carry tool-result errors through the provider protocol instead of stdout text-sniffing
- filed: fabro-b09c
- Change: propagate the tool-result error flag through the agent-session/provider protocol layer (lib/components/fabro-workflow agent session, pebble Environment protocol) where the provider supports it; surface an explicit failure marker where it cannot. Effect: deterministic error signaling for failed tool calls, removing one blind-retry class across all agent stages. Evidence: seq 108-110 failed ml record round-tripped without error flag; seq 113-155 unsupported_control warnings.

### Backfill pull_request.state for terminal-but-unmerged runs in the fabro_runs_list projection
- filed: fabro-4bb7
- Change: populate pull_request.state in the fabro_runs_list run projection for terminal runs whose PR is still unmerged. Effect: the planner's in-flight guard stops running blind exactly when a PR sits unmerged (fabro-91ff duplicate-claim class). Evidence: seq 33 saw run 01M2K3NQEW succeeded with PR #153 state: null, degraded "no exclusions" mode.
