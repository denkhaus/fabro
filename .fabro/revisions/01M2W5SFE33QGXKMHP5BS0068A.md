# Revision — run 01M2W5SFE33QGXKMHP5BS0068A

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2W5SFE33QGXKMHP5BS0068A.md
- seeds filed: none — zero filing credit this pass (0 same-pass stale/superseded closes); all surviving findings journaled as overflow
- balance: 0 non-exempt seeds filed / 0 — no credit this pass
- basis: run 01M2W5SFE33QGXKMHP5BS0068A, workflow version 341257a3c3ccf500ad9774c1dab92b6aef751603b31e66b83b7c48131841f51b, commit 3df6f2a186436637bc9a466105fdc907d3841b01
- revised_at_commit: 3df6f2a186436637bc9a466105fdc907d3841b01 (ADR-0015: engine drift signal for later judgement)

## Findings

### Repair the FABRO_VALIDATE_PREBUILT fast path inside the run sandbox
- filed: overflow to journal (no filing credit this pass)
- change: verify and repair the fast path in `scripts/validate-workflows.nu` and the baked `.fabro/bin/fabro-validate` in toolchain image `99c855a689b3`; expected effect: about −2 min wall on every workflow-authoring seed (133s of 138s tool time burned in this run's implementer stage).

### Make set-but-empty fabro_tools mean deny in the engine
- filed: overflow to journal (no filing credit this pass)
- change: presence-sensitive parsing in `lib/foundation/fabro-types/src/graph.rs` (split_key_list near line 436) plus `stage_tools` in `lib/components/fabro-workflow/src/handler/llm/pebble.rs`; expected effect: the graph attribute becomes mechanically load-bearing, closing the capability-leak class (implementer paid 340s inference on this sharp edge).

### Flag upstream-blocked candidates in planner-preflight verdicts
- filed: overflow to journal (no filing credit this pass)
- change: `.fabro/workflows/develop/scripts/planner-preflight.nu` flags candidates whose body carries an upstream-PR-open, close-when-merged status; expected effect: ~20–30s and one tool call saved per planning pass, removes the recurring top-of-queue distraction.
