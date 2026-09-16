# Revision — run 01M2MY1JY4MPAXD9KWFN675385

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2MY1JY4MPAXD9KWFN675385.md
- seeds filed: fabro-c643 — Warm the develop run container's Rust build (cargo-chef layers in run toolchain image or persistent sccache volume)
- basis: run 01M2MY1JY4MPAXD9KWFN675385, workflow version 6ad783be021e539bec3ef20d5ab0773111d20471eab6b0892bc38e48239d1d19, commit 5a0d154ac2a9ae7c712c8587bfb91641b130eb84
- revised_at_commit: 5a0d154ac2a9ae7c712c8587bfb91641b130eb84 (ADR-0015: engine drift signal for later judgement)

## Findings

### Warm the develop run container's Rust build: cargo-chef dependency layers in the run toolchain image or a persistent sccache volume

- filed: fabro-c643 (capability-affecting: `needs-user,revision` labels per ADR-0019 — changes `.fabro/Dockerfile.toolchain` / `run.environment` agent-reachable surfaces; implementation awaits explicit user approval)
- concrete change: bake cargo-chef dependency layers into the run toolchain image, or mount a persistent sccache volume via `run.environment` in `.fabro/workflows/develop/workflow.fabro`
- evidence: ~15 of 20.3 min wall was Rust compilation in a fresh container (implementer 556 s + tester 340 s), `lifecycle.preserve=false`, no cross-run cache
- expected effect: 4-7 min off every develop run's wall time
- boundaries checked: `fabro-cfd6` (CI dogfood-gate image only), `fabro-fe15` (debug-CLI shared cargo cache), `fabro-581b` (mold linker) — no duplicates, no supersessions
