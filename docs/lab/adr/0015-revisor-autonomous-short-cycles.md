# Revisor v2: autonomous short cycles

Decided 2026-09-05 (grill-with-docs session); supersedes the gate-control
part of ADR-0011 and refines the cycle mechanics of ADR-0013 phase 2.

## Context

The revisor v1 loop walks the whole unrevised backlog in one invocation,
oldest-first, with a HITL approval gate after every revision cycle. Two
problems surfaced in practice: the gate serializes the loop on a human,
and oldest-first revises runs whose workflow definitions no longer match
the current tree, producing seeds with an outdated basis that burden the
tracker. 14 workflow seeds from v1 passes required a full manual triage
(2026-09-05); six were already implemented, one was partially done.

The user direction for the self-evolving loop: trust the process, keep
humans out of the loop, and control quality by discarding stale seeds
*before* implementation rather than approving filings after the fact.

## Decision

- **No HITL gate.** The `approve` hexagon is removed. Control moves to
  the develop side: seeds can always be discarded before implementation.
  Filing merges via the normal PR + auto-merge path, identical to develop.
- **Newest-first selection.** `select` picks the NEWEST unrevised terminal
  develop run. Backlog ordering is owned by invocation frequency: a
  frequent revisor works on fresh evidence; a rare one drains the backlog.
  No tombstones for old runs.
- **Configurable pass size.** A graph attribute `revisions_per_pass`
  (default 1) bounds how many revisions one invocation performs before
  exiting naturally. CLI override comes with the 24/7 host.
- **Freshness precondition (hard).** A run is *revisable* only when its
  `workflow_version_id` equals the current registered develop workflow
  version — runs whose workflow definition drifted are stale-evidence and
  must not produce seeds. Engine prerequisite: `fabro_runs_list` exposes
  `workflow_version_id` (same pattern as `sandbox_available`).
- **Engine drift (soft).** Engine changes between run and revision are
  recorded as a journal observation and as `revised_at_commit` in the
  revision marker, not as a hard block. Strict repo-commit pinning is
  revisited when the dedicated host raises cadence.
- **Serialization principle.** Own features, upstream merges, and
  processing revisor seeds run in ONE line. Concurrent changes invalidate
  evidence and seeds; parallelism here works against effectiveness.
- **Self-cleaning seeds.** Every seed body records a basis line
  (`run id`, `workflow version id`, `repo commit`). The develop planner
  runs a stale-basis check before claiming a seed and closes moot seeds
  as `superseded` (ADR-0014 vocabulary). The `file` node duplicates-guards
  via `sd search` before `sd create`.
- **Transition.** All workflows start manually until the 24/7 host
  exists; `just cycle` chains develop -> terminal -> revisor in one
  command. ADR-0013 qualification cycles count from this v2 form.

## Consequences

- The revisor becomes safe to schedule (cron later) and to leave
  unattended; its runtime per invocation is bounded by
  `revisions_per_pass`.
- Seed quality is enforced at consumption time (planner) instead of
  production time (gate) — matches the self-evolving direction.
- The web UI files tab still refuses large diffs (`.seeds/issues.jsonl`:
  "too large to render inline"); a seed covers the truncated/per-record
  diff rendering. Tracker-file diffs stay unreadable in the UI until then.
