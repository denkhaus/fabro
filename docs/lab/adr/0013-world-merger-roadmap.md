# World merger roadmap: revisor first, migrate once

Decided 2026-08-27 (user accepted the recommendation); builds on ADR-0011
(Workflow-Revisor) and ADR-0012 (dogfooding).

## Context

Three worlds exist today: `denkhaus` (platform), `denkhaus-lab` (gofib
product + develop workflow), `meta/denkhaus-lab` (retired; durable home in
`docs/lab/`). The goal is one world: fabro develops fabro, the develop
workflow runs on the fabro repo, `denkhaus-lab` and the meta branch die.

Two orders were weighed: polish develop to perfection, migrate, then build
the revisor — or build the revisor first, let it drive develop quality,
migrate once. The decisive facts: the revisor's engine work is
world-independent; develop is already measured good (3/3 first-pass
approvals, stable preambles) and its remaining polish items are workflow
files that migrate for free; gofib runs cost ~$1/16min while Rust-product
runs will cost far more; the migration itself is the most turbulent phase
and benefits most from an operating self-improvement loop.

## Decision

- **Revisor first, on the lab world as training ground.** Build order:
  engine prerequisites (run_logs tool, ask/list tools with revises scope)
  → develop polish items in parallel (small, transferable) → the revisor
  workflow itself on `denkhaus-lab` (manual starts, quality ramp).
- **"Develop is good enough" is proven BY the revisor loop**, not by
  hand-polishing: at least TWO full revisor cycles on the lab world in
  which the revisor files seeds that develop implements successfully —
  the closed meta-loop.
- **Migrate ONCE at the end:** develop + revisor move together onto the
  merged world; fabro becomes the product; the revisor burns down
  migration friction. Then `denkhaus-lab` and `meta/denkhaus-lab` retire
  and the revisor goes cron.
- **Trackers consolidate** into one product tracker on the merged world
  (ADR-0012); platform/engine seeds move with it.

## Consequences

- The migration seed lists the hard prerequisites (Rust quality gate in
  the sandbox image — adjacent to fabro-199a build-context —, tracker
  consolidation, prompt/evidence adaptation for Rust diffs); they stay
  visible while the revisor is built.
- Upstream posture unchanged; the merger is ours, not an offer.


## Phase 3 executed (2026-09-05)

Qualification 2/2 green (cycles: fabro-37a6 via 01M1RF53QW, fabro-56c2
via 01M1RGQHJ5 + revisor 01M1RH2KM0). Executed decisions:

- **Worlds retired.** `denkhaus-lab` and `meta/denkhaus-lab` deleted
  after final-state tags `archive/denkhaus-lab-final` and
  `archive/meta-denkhaus-lab-final`; trackers were empty, protections
  removed, the dangling test automation deleted. The real work of the
  merger lives on `denkhaus`.
- **Conductor replaces cron-per-workflow.** One engine-side `conductor`
  workflow (UI cron automation) is the serialized line: a pass runs
  EITHER an upstream merge (threshold >= 5 new upstream commits —
  single-commit drift must not consume merge slots) as an infra-only
  pass OR one develop+revisor cycle. Children are created via
  parent-scoped `fabro_run_create` only after the previous child's PR
  auto-merged; serialization is structural (create-after-merge), not
  temporal. `merge-upstream` is its own workflow (different lifecycle,
  gate scope, and deploy handoff: runs push, the HOST deploys).
- **429/budget exhaustion**: LLM retries are exponential with
  Retry-After (fabro-llm); a persistent 429 fails the pass and the cron
  is the deterministic reviver — next fire retries on a reset budget.
- **Auto-merge prerequisite**: `dogfood-gate` (path-adaptive CI) is the
  only required check; the host-posted `lab-check-local` was dropped
  because the conductor cannot post it, and failed runs never receive
  PRs (engine creates PRs only on terminal success), so the gate alone
  cannot merge garbage.
- **Agent end-state**: the agent session is the control function —
  revisor-revision each cycle, seeds for gaps, deploy on merge reports
  (workstation), grilling at forks.
