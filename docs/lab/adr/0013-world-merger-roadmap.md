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
