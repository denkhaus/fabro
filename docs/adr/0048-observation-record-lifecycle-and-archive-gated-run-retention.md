# ADR-0048: Observation record lifecycle and archive-gated run retention

## Status

Decided 2026-11-20 (operator grill; amends ADR-0038's append-only
record semantics and narrows the fork.34-era run retention posture).

## Context

ADR-0038 made stage observations platform records: append-only,
durable, no delete path anywhere in the store, the run-removal flow,
or the prune tooling. Three weeks of the required-presence contract
(2026-11-20: 873 records, ~44/day) grow the table unbounded.

The run retention pass (`max_run_age_days=7`, 24h cadence) deletes
EVERY terminal run older than the floor — no archive distinction —
gated by the work#1299 triage clock: a run carrying ANY untriaged
observation is spared indefinitely. The operator's 2026-11-20 review
surfaced the drift between that behavior and the intended posture:
runs must NOT be auto-deleted wholesale — the archive gesture must be
the operator-controlled drain — and the growth concern is answered by
that drain, not by an unbounded append-only store.

The sandbox/image reap is a separate arm already (`terminal_sandbox_age_hours`
floor, 15-min cadence, work#168's disk clock): storage dies young,
metadata survival is the operator-visible artifact.

## Decision

1. **Observation records travel with their run.** Deleting a run
   (operator `rm` or the retention pass) deletes its observation
   records. No tombstone history: the durable value of a triage
   decision is the work-item link — the filing IS the decision
   (ADR-0044), and the linked item's own history carries the outcome.
2. **The retention pass becomes archive-gated.** Only ARCHIVED runs
   older than `max_run_age_days` are auto-deleted. Non-archived
   terminal runs keep their metadata forever. Archiving is the
   operator-controlled drain: reviewed = deletable.
3. **The work#1299 triage gate stays absolute.** An untriaged
   observation spares its run indefinitely; the triager's incidence is
   the guarantee, not a TTL.
4. **The manual CLI prune keeps its full power** (`fabro system
   prune`): operator explicitness beats policy; only the SCHEDULED
   pass is archive-gated.
5. **The sandbox/image reap is unchanged** (4h floor, independent
   arm).

## Consequences

- The observation store converges to: records of non-archived runs
  (operator-paced) plus `max_run_age_days` of archived-run records.
  Growth is operator-paced, not unbounded-by-design.
- Work-item enrichment on observation links (item_state, work#1237)
  loses its data source when a record dies; the work table reads the
  raw tracker state (work#1511's client unification) and is
  unaffected.
- Deleted records are not a lost audit trail: the run's summary, the
  linked work items, and their close comments carry the durable
  history (verified in practice: seven already-landed park
  adjudications on 2026-11-20 each cited the full per-criterion
  evidence into the item's close comment before the runs were
  deleted).
- Transition: `max_run_age_days=0` (retention disabled) applies via
  tofu until the archive-gated pass deploys; the configured value
  returns with that deploy.
