# Revision — run 01M2V6WCJ7JV7AJZ90NSSE00X6

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2V6WCJ7JV7AJZ90NSSE00X6.md
- seeds filed: fabro-ab93 — Gate planner-preflight in_flight marker on run/PR terminality; fabro-4b3d — Document fetch-then-FETCH_HEAD upstream-diff form in develop PROJECT_FACTS
- basis: run 01M2V6WCJ7JV7AJZ90NSSE00X6, workflow version 8dafcf4ebc5c1b1a4781f85b7f5c1a6125f166bfcdef6bbc34414c18e13aa5f3, commit b1a080d6380a7cda29d4d78b620c444f28d50ec1
- revised_at_commit: b1a080d6380a7cda29d4d78b620c444f28d50ec1 (ADR-0015: engine drift signal for later judgement)

## Findings

### Gate planner-preflight in_flight marker on run/PR terminality
- filed: fabro-ab93
- Change: in `.fabro/workflows/develop/scripts/planner-preflight.nu`, only mark a seed `in_flight` when the referenced run is non-terminal OR its PR state is open/unknown-and-branch-present. Effect: `in_flight` becomes trustworthy and the planner skips the ~60s adjudication lap the stale marker caused (run events seq 44-83, PR #242 rejected, branch gone).

### Document fetch-then-FETCH_HEAD upstream-diff form in develop PROJECT_FACTS
- filed: fabro-4b3d
- Change: add a PROJECT_FACTS bullet stating the sandbox has no `upstream/main` ref and upstream comparisons must use `git fetch origin main` then diff against `FETCH_HEAD`. Effect: fork-strategy seeds stop rediscovering the workaround and lose a guaranteed failed shell call plus recovery turn.
