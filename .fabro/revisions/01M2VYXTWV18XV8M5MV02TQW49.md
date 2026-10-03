# Revision — run 01M2VYXTWV18XV8M5MV02TQW49

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2VYXTWV18XV8M5MV02TQW49.md
- seeds filed: none — 0 credit this pass (ADR-0022); consolidated finding journaled as overflow
- balance: 0 non-exempt seeds filed / 0 — no same-pass stale-superseded closes
- basis: run 01M2VYXTWV18XV8M5MV02TQW49, workflow version 341257a3c3ccf500ad9774c1dab92b6aef751603b31e66b83b7c48131841f51b, commit e0589cd2f4220f3721dc4231bf5b882403d77dda
- revised_at_commit: e0589cd2f4220f3721dc4231bf5b882403d77dda (ADR-0015: engine drift signal for later judgement)

## Findings

Both findings target `.fabro/workflows/develop/scripts/planner-preflight.nu` and were
consolidated into ONE multi-arm seed candidate (fabro-ae74 schema). Dedupe ran clean
(`sd search`: upstream, report-only, dry-run — no seed names either concrete change;
`fabro-ab93` gates the `in_flight` marker, `fabro-8678` gates closure evidence — both
different mechanisms). ADR-0022 balance: 0 credit this pass (no same-pass stale/superseded
closes), so the consolidated candidate was NOT filed and rides this marker + the pass
journal as overflow for the next pass to re-file against its own balance.

### overflow: multi-arm planner-preflight.nu seed — mechanically skip upstream-gated seeds; gate report-only route on resolvable sha

- arm 1 — Skip upstream-gated seeds mechanically: parse a `UPSTREAM PR OPEN: <repo>#<n>`
  line from candidate seed bodies, check whether the fix commit is an ancestor of
  `origin/denkhaus`, mark the row `upstream_gated: true` (mirroring `in_flight`) so the
  planner skips it mechanically. Grounded in this run: the planner burned ~60 s and
  several LLM rounds re-adjudicating `fabro-af22` (git-log greps, skill-reference
  searches, a failed `origin/main` probe) only to conclude "upstream-gated, skip"; its
  journal painpoint notes this recurs every lap. Expected effect: −60–90 s and
  −$0.10–0.15 per develop lap, recurring.
- arm 2 — Gate report-only route on resolvable sha: gate the route expression on a
  resolvable closing sha for the top candidate in both live and `--report-only` modes.
  Grounded in this run's reviewer journal (observation 3): in `--report-only` mode a top
  candidate with verdict `duplicate` and null sha still routes "Already landed" with
  `closed.seed: null` — the ambiguity guard from `fabro-ead4` (PR #262) covers only the
  live close arm, and this run's implementer dry-run exercised exactly the report-only
  mode. Expected effect: dry-run routing matches live semantics; prevents a
  dry-run-shaped exit without a close.

Next pass: re-run dedupe, then file this as ONE seed with both arms, priority 1, labels
revision, basis line citing that pass's run and commit.
