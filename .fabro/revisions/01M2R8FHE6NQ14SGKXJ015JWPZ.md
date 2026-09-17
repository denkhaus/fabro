# Revision — run 01M2R8FHE6NQ14SGKXJ015JWPZ

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2R8FHE6NQ14SGKXJ015JWPZ.md
- seeds filed: fabro-a1ef — Implementer hard rule: force a rebuild before re-diagnosing a fix-surviving test failure
- seeds filed: fabro-7daf — Planner-preflight: verify cited file:line anchor content, not only landed-commit greps
- basis: run 01M2R8FHE6NQ14SGKXJ015JWPZ, workflow version f4ab9ff48b6e162ee81f42689c29e27253fba4fe3632992c2e164afa31eb5ca5, commit c68bc06b092a117bb60d1119c2261a0d14c570fd
- revised_at_commit: c68bc06b092a117bb60d1119c2261a0d14c570fd (ADR-0015: engine drift signal for later judgement)

## Findings

### Implementer hard rule: force a rebuild before re-diagnosing a fix-surviving test failure
- filed: fabro-a1ef
- change: one new hard rule in `.fabro/workflows/develop/prompts/implementer.md` — a test failure that survives an obvious fix triggers a forced rebuild (`touch <file>` / `cargo clean -p <crate>`) before re-diagnosis. Expected effect: removes a recurring stale-artifact trap that costs a wasted re-run plus wrong-root-cause risk (run evidence: old panic text persisted 1.2 s after the fix; forced rebuild cleared it).

### Planner-preflight: verify cited file:line anchor content, not only landed-commit greps
- filed: fabro-7daf
- change: extend `.fabro/workflows/develop/scripts/planner-preflight.nu` to check that cited `file:line` anchors still contain their claimed content and flag mismatches in the verdict table. Expected effect: dead seeds (rotted anchors, nonexistent nodes) get a sub-second deterministic verdict instead of a ~42 s planner reasoning lap and stop occupying top priority slots.
