# Revision — run 01M1Z1PK3D7R9MEWDP7QDZMA1D

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M1Z1PK3D7R9MEWDP7QDZMA1D.md
- seeds filed: fabro-9e49 — Make current_seed_brief criteria newline-bulleted in the planner prompt
- basis: run 01M1Z1PK3D7R9MEWDP7QDZMA1D, workflow version be58840fab1c246bee538760df54ab8f8bfe6bb0bcf0356df6aaf38d5f14ef20, commit ce495789f5be8fbe233670d0e0c3a72ddd9d4bca
- revised_at_commit: ce495789f5be8fbe233670d0e0c3a72ddd9d4bca (ADR-0015: engine drift signal for later judgement)

## Findings

### Make current_seed_brief criteria newline-bulleted in the planner prompt
- filed: fabro-9e49
- Change: in `.fabro/workflows/develop/prompts/planner.md` step 6, require newline-separated bullets (`\n- `) inside the `current_seed_brief` JSON string instead of one mashed line. Basis: run 01M1Z1PK3D7R9MEWDP7QDZMA1D (seed fabro-cf76) — the planner (seq 62) emitted all six acceptance criteria on one line despite the bulleted-criteria instruction, forcing the implementer (seq 107) to re-split them for the PASS/FAIL report.
- Expected effect: clean criterion→PASS/FAIL mapping for implementer and reviewer; fewer ambiguity-driven Changes-requested cycles (~2.5 min + ~$0.15 each). Thematic overlap only (not supersession) with fabro-ad1b, fabro-b8ed, fabro-fb19 — those target implementer-side citation/brief content; nothing closed.
