# Revision — run 01M1YS4NJH27NS7CQ02AWM9BNE

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M1YS4NJH27NS7CQ02AWM9BNE.md
- seeds filed: fabro-05d0 — Bake gh into the develop toolchain image (or check open PRs engine-side) so the in-flight-PR guard functions; fabro-5b0a — Surface degraded fail-open planner checks as run-level warnings in the run summary and PR body
- basis: run 01M1YS4NJH27NS7CQ02AWM9BNE, workflow version ae2869aca84cea61832e77a2926b97ebb79d70300014555c724f2862e12612c9, commit 774f67e9548357656f837bd152c22be72920eb1e
- revised_at_commit: 774f67e9548357656f837bd152c22be72920eb1e (ADR-0015: engine drift signal for later judgement)

## Findings

### Bake gh into the develop toolchain image (or check open PRs engine-side) so the in-flight-PR guard functions

- filed: fabro-05d0
- The planner's mandated `gh pr list --state open` failed with exit 127 (`gh: command not found`, events seq 40-41) and degraded fail-open, structurally disabling the double-pick guard from fabro-d0c7 on `fabro-toolchain:noble` even though the engine has GitHub access (PR #51 was created). Change: add `gh` to the toolchain image built from `.fabro/Dockerfile`, or move the open-PR check engine-side using run PR metadata. Expected effect: the guard actually audits double-picks and one guaranteed erroring tool call disappears from every planning pass.

### Surface degraded fail-open planner checks as run-level warnings in the run summary and PR body

- filed: fabro-5b0a
- The gh open-PR check degraded fail-open but the degradation was visible only in a journal line; a human reading PR #51 cannot distinguish "succeeded" from "succeeded with guards down". Change: engine-side, emit a run-level warning surfaced in the run summary and PR body whenever a mandated planner check degrades fail-open. Expected effect: guard-down runs become human-visible at review time instead of silently passing.
