# Revision — run 01M23KJM3SM70S2S6QSSEWSJPY

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M23KJM3SM70S2S6QSSEWSJPY.md
- seeds filed: fabro-f7cf — Planner step 7: rewrite gate-command verification criteria to defer to the deterministic tester
- basis: run 01M23KJM3SM70S2S6QSSEWSJPY, workflow version 70cee3b32d01c25022b0c84ba4b42f2e8c93d679f400542db4ca1cafafa1a6c1, commit b7734297f0f36c3255f2d599ed2f7feb161f1c00
- revised_at_commit: b7734297f0f36c3255f2d599ed2f7feb161f1c00 (ADR-0015: engine drift signal for later judgement)

## Findings

### Planner step 7: rewrite gate-command verification criteria to defer to the deterministic tester — filed fabro-f7cf

The claimed seed's verification bullet said "then `just qualitygate` per PROJECT_FACTS"; the implementer ran it (seq 111-113, 4.2 s) and the deterministic tester then ran the byte-identical command on the unchanged tree (seq 128-129, 4.3 s) — the implementer journaled the contradiction against `implementer.md` step 4's own gate ban. Change `.fabro/workflows/develop/prompts/planner.md` step 7 (contradiction check): add a rule that any verification criterion naming the PROJECT_FACTS gate command is rewritten to "gate green via the deterministic tester step" before the brief ships. Expected effect: eliminates the double gate entirely (~4 s wasted here; up to ~15 min cold-cache build on Rust-touched seeds, per the 8m46s build cited in `workflow.fabro`).
