# Revision — run 01M23W4S7Q2GNEVPY779M4GMN6

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M23W4S7Q2GNEVPY779M4GMN6.md
- seeds filed: fabro-c49c — Close the gate-ban equivalence loophole in planner step 7; fabro-5e3c — Fix hardcoded cwd restore in evidence-smoke.nu
- basis: run 01M23W4S7Q2GNEVPY779M4GMN6, workflow version 38bab0b8b1faad62aaddb5e68facf6f1c58286169676479d3edbf99d4205b91c, commit 4c8683243f8c295438cf019e2f061f8812fc58db
- revised_at_commit: 4c8683243f8c295438cf019e2f061f8812fc58db (ADR-0015: engine drift signal for later judgement)

## Findings

### Close the gate-ban equivalence loophole in planner step 7
- filed: fabro-c49c
- Change: in `.fabro/workflows/develop/prompts/planner.md` step 7, extend the gate-command rewrite rule from matching only `just qualitygate` to also matching its recipe body or equivalent invocation (per PROJECT_FACTS the recipe delegates to `nu scripts/qualitygate.nu`); the permitted substitute stays gate-green via the deterministic tester step. Evidence: in this run the planner rewrote the banned command into its byte-equivalent body (checkpoint seq 82, `justfile:128`), so the implementer and reviewer each ran the full gate on top of the tester — three gate executions on one tree. The prior fix (fabro-f7cf, closed and implemented) only matched the literal command name; this is the bypass of that rule, a different mechanism — not a duplicate. Expected effect: no redundant full-gate runs; on Rust-touching seeds each avoided run is a cold compile measured at 8-15 min.

### Fix hardcoded cwd restore in evidence-smoke.nu
- filed: fabro-5e3c
- Change: in `scripts/evidence-smoke.nu` (~line 60), replace the hardcoded `cd /workspace/fabro` cwd restore with a captured `$env.PWD` at script start. Evidence: the reviewer in this run found the defect (journal seq 296) and approved anyway because the only channel was a full Changes-requested cycle; the smoke was checked into the gate by seed fabro-bfe1's new loop-asset tier (open; thematic overlap, not the same change — cross-referenced, nothing closed) and now executes in every future `just qualitygate`, misbehaving in any worktree checkout. Expected effect: the new gate tier stays green outside the canonical checkout path, and the latent break stops executing on every run.
