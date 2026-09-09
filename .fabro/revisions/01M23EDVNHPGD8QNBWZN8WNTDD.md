# Revision — run 01M23EDVNHPGD8QNBWZN8WNTDD

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M23EDVNHPGD8QNBWZN8WNTDD.md
- seeds filed:
  - fabro-c4ce — Scope implementer nextest to test-file-touched crates only
  - fabro-0586 — Add an available-binaries line to develop PROJECT_FACTS
- basis: run 01M23EDVNHPGD8QNBWZN8WNTDD, workflow version 70cee3b32d01c25022b0c84ba4b42f2e8c93d679f400542db4ca1cafafa1a6c1, commit 1bd14a4d41278a40a3615a61ffe55368fe0959e2
- revised_at_commit: 1bd14a4d41278a40a3615a61ffe55368fe0959e2 (ADR-0015: engine drift signal for later judgement)

## Findings

### Scope implementer nextest to test-file-touched crates only (fabro-c4ce)
Make step 4 test scoping in `.fabro/workflows/develop/prompts/implementer.md` mechanical: nextest only for crates whose test files the seed touched; fmt and clippy for every code-touched crate. In this run the implementer ran the full suite on all five touched crates (3,370 tests) although tests were added only in `lib/components/fabro-github` and `lib/apps/fabro-server`, and the tester gate deterministically re-ran the identical five-crate commands (227 s) anyway. Refines closed seed fabro-fc1b, whose rule was over-applied here. Expected effect: large cut of implementer wall and cost (30.1 min / $2.04 of $2.20 total, 1,092 s tool time) on every multi-crate Rust seed with gate coverage unchanged.

### Add an available-binaries line to develop PROJECT_FACTS (fabro-0586)
Add one line to `.fabro/workflows/develop/prompts/project-facts.md` stating which shell binaries the run toolchain provides (rg, sd, ml, just, nu — no fd). The planner lost a call to `fd: command not found` (event 46) and the implementer guessed a wrong crate directory before finding `lib/components/` (event 99). Crate-to-path-map half is open seed fabro-3d2d; this seed covers only the tool-availability line. Expected effect: removes binary-probing calls in planner and implementer on every run.
