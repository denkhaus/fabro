# Revision — run 01M0WWKAQCWZC0Q0JK019H0ZC7

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M0WWKAQCWZC0Q0JK019H0ZC7.md
- seeds filed:
  - fabro-6a5a — Fix PR postlude: disable auto-merge and stop the failing PR-body model call
  - fabro-e702 — Require timeout_ms on compile/test shells and ban placeholder appends in implementer prompt
  - fabro-8d2c — Raise preamble_budget_kb from 24 to 32 — evidence blob detour persists
  - fabro-a23f — Make implementer journal report errored or timed-out tool calls
  - fabro-f4cd — Fast-path planner to sd show when the goal names a seed id
  - fabro-f8f8 — Derive files_touched from the stage git diff, not write-tool calls

## Findings

### Fix PR postlude: disable auto-merge and stop the failing PR-body model call
Filed as fabro-6a5a (partial overlap with open fabro-9f97, which covers the journal-only-diff case; this covers real-diff runs). Change: set `auto_merge=false` in the run-spec pull_request block and fix/disable the PR-body LLM call that fails JSON parsing. Expected effect: ~50s faster run completion, two fewer recurring warn-level failures.

### Require timeout_ms on compile/test shells and ban placeholder appends in implementer prompt
Filed as fabro-e702. Change: two hard rules in `.fabro/workflows/develop/prompts/implementer.md` — compile/test shells must pass `timeout_ms>=60000`, and no placeholder appends. Expected effect: implementer pass ~100s instead of ~150s; silent-timeout failure class eliminated.

### Raise preamble_budget_kb from 24 to 32 — the evidence blob detour is back
Filed as fabro-8d2c — SUPERSEDES the targets of open fabro-83f9 and fabro-35ab (both still say raise from 12); those should be closed/amended rather than implemented separately. Change: `preamble_budget_kb=32` in `.fabro/workflows/develop/workflow.fabro`. Expected effect: evidence arrives inline in the reviewer preamble, removing one read_file round-trip and the preview-misread false-rejection mode.

### Make implementer journal report errored or timed-out tool calls
Filed as fabro-a23f (complements open fabro-b809). Change: Journal section of the implementer prompt must always record errored/timed-out calls in observations. Expected effect: friction self-reports on the run where it happens.

### Fast-path planner to sd show when the goal names a seed id
Filed as fabro-f4cd (overlaps the tracker-empty case in open fabro-b382 and fabro-e4fa; covers the named-seed case). Change: planner prompt step 1 routes named-seed goals straight to `sd show`, bans redundant `sd list` re-confirmation. Expected effect: two fewer shell calls and one LLM turn on the common named-seed case.

### Derive files_touched from the stage git diff, not write-tool calls
Filed as fabro-f8f8 (engine-side). Change: compute stage `files_touched` from the stage checkpoint git diff. Expected effect: accurate change attribution even for shell-edited files.
