# Revision — run 01M2YMRJQZMC6SVTQJ3JC49CWC

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2YMRJQZMC6SVTQJ3JC49CWC.md
- seeds filed: none — zero balance credit this pass (no same-pass stale/superseded closes); all surviving findings journaled as overflow below
- balance: 0 non-exempt seeds filed / 0 — no credit this pass
- basis: run 01M2YMRJQZMC6SVTQJ3JC49CWC, workflow version fe511dfedb1194e5c079494a94f189fc43c9acb68d7bbae397dde2d30f3e115a, commit 16e2ea04e6f0a98a4f996e0c260a21d3b0e5bcbc
- revised_at_commit: 16e2ea04e6f0a98a4f996e0c260a21d3b0e5bcbc (ADR-0015: engine drift signal for later judgement)

## Findings

### 1. Upstream-gated verdict arm in planner-preflight.nu — overflow-dup
Theme already open in the ledger — `overflow-dup: Park upstream-PR-gated seeds mechanically in the planner preflight (open in 01M2VVFMSQXHK0E3SNV2GNTP43.md:14)`. This pass's evidence strengthens it: run 01M2YMRJQZMC6SVTQJ3JC49CWC planner seq 53–82 burned ~40s/~$0.12 re-deriving the `fabro-af22` gate on upstream PR fabro-sh/fabro#784. Proposed arm shape: per-candidate `upstream_gated` flag from a mechanical body-marker grep (`UPSTREAM PR OPEN`, `close when #<n> is merged`); planner skips flagged candidates like `in_flight`. Effect: ~-40s/~$0.12 per develop run; arm costs milliseconds.

### 2. Flag out-of-repo change targets at preclaim/intake — overflow (new)
- overflow: Flag seeds whose change target lives outside the repo at preclaim/intake — add an out-of-repo target arm to `.fabro/workflows/develop/scripts/planner-preflight.nu` (same verdict table as the upstream-gated arm) or the revisor intake lint: a seed whose named change target (binary/path) has no source in this repo gets flagged and receives the fabro-a285 blocked-external refile; apply it to `fabro-23d4` now; effect: removes ~31s/run (run 01M2YMRJQZMC6SVTQJ3JC49CWC planner seq 83–100 discovered `sd note` targets the external npm CLI at `/mise/bun-global/bin/sd`, then journaled a refile recommendation that died with the run) and prevents recurrence for future external-tool seeds. Not covered by open `fabro-4c81` (stale repo paths) or `fabro-a285` (one-off refile).

### 3. Brief consistency criteria must enumerate their full mirror surface — overflow (new)
- overflow: Require brief consistency criteria to enumerate their full mirror surface — in `.fabro/workflows/develop/prompts/planner.md` steps 6/7, a consistency criterion must either enumerate its full search surface (e.g. 'develop prompts + the revisor `file.md` mirror') or carry a parenthetical matching the criterion scope, never contradictory scopes like 'any other prompt or doc passage (grep the develop prompts)'; effect: removes ~$0.03–0.05 of ambiguity cost per prompt-mirror seed across implementer and reviewer (run 01M2YMRJQZMC6SVTQJ3JC49CWC: 43s/2,483-reasoning-token implementer burst on `.fabro/workflows/revisor/prompts/file.md:93` scope, plus reviewer re-adjudication). Not covered by `fabro-dad8`/`fabro-7773` (probe/seed-body contradictions) or closed `fabro-a18d`.

### 4. Allowlist by-design routing-schema and date-pin warnings in prompt-lint.nu — overflow-dup
Theme already open in the ledger — `overflow-dup: Silence by-design prompt-lint warnings in the qualitygate (open in 01M2XQJC3TWRMJ1128WRXRQYXH.md:20)`. This pass's evidence strengthens it: run 01M2YMRJQZMC6SVTQJ3JC49CWC gate output carried 11 of 11 by-design warnings, so a real schema regression is invisible in the noise on every gate run. Concrete arm: intentional-routing marker (e.g. `"x-routing-intent": true`) silencing the routing-named-property warning for `planner-output.schema.json` and `develop-output.schema.json`, plus a load-bearing annotation silencing the `.fabro/project-facts.md` date-pin '>45 days' warning in `.fabro/scripts/prompt-lint.nu`. Not covered by closed `fabro-a211` (introduced the warnings, no allowlist) or open `fabro-8275` (different warn, engine module).
