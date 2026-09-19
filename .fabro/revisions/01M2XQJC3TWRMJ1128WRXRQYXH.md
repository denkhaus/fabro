# Revision — run 01M2XQJC3TWRMJ1128WRXRQYXH

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2XQJC3TWRMJ1128WRXRQYXH.md
- seeds filed: none — zero balance credit this pass (no same-pass stale/superseded closes)
- balance: 0 non-exempt seeds filed / 0 — no credit this pass
- basis: run 01M2XQJC3TWRMJ1128WRXRQYXH, workflow version 0de6e944c9eadbd3b23dbc46db9a4022b82e686cde7fe54384a6f59be7722668, commit 8da59dd937f52e3f190aafb4c96db6767553f4bc
- revised_at_commit: 8da59dd937f52e3f190aafb4c96db6767553f4bc (ADR-0015: engine drift signal for later judgement)

## Findings

### 1. Park externally-blocked seeds in the planner preflight

- result: overflow-dup: Park upstream-PR-gated seeds mechanically in the planner preflight (open in `.fabro/revisions/01M2VVFMSQXHK0E3SNV2GNTP43.md`)
- concrete change: add an `externally_blocked` arm to `.fabro/workflows/develop/scripts/planner-preflight.nu` marking candidates whose body names an upstream PR/rev or a needs-upstream label; expected effect: removes ~50s / 7 tool calls of re-derivation per run (evidence: fabro-af22 re-proved upstream PR #784 block, seq ~60).
- also links thematically to the open `file ONE multi-arm seed (fabro-ae74 schema) for planner-preflight.nu` entry (`.fabro/revisions/01M2WCNY3WGP4QFZ24KPACBTND.md`) and the memoized skip-adjudication entry (`.fabro/revisions/01M2X8458MDWMRVBRVEMDX9W4J.md`); a next pass with credit should consolidate all preflight arms into one multi-arm seed.

### 2. Silence by-design prompt-lint warnings in the qualitygate

- overflow: Silence by-design prompt-lint warnings in the qualitygate — `scripts/qualitygate.nu` prompt-lint allowlists routing-intent schemas (title/description-declared, e.g. `planner-output.schema.json`, conductor schema per fabro-9ec3) and the date-pin "older than 45 days" nag for the pinned toolchain; effect: only genuine anomalies surface instead of 11 per-run warnings that train readers to skip them. Dedupe verified this pass (`sd search` on qualitygate / lint / warnings: no seed covers lint-noise allowlisting; fabro-f18a is tool-call JSON lint, closed fabro-a211 documented the routing contract only). NEXT PASS: re-run dedupe, then file.
