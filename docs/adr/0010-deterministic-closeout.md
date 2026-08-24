# Post-approval bookkeeping is deterministic — the closeout step

## Status

Accepted (2026-08-24). Grounded in the first fully-autonomous run (review of
run with the first successful PR auto-merge).

## Context

After a reviewer approved, the graph routed back to the planner, whose next
visit existed only to run `sd close <seed>` and re-read the tracker
(`sd ready`/`sd list`) — mechanically decidable actions. That visit cost a
full LLM session: 21s, $0.021, ~7% of wall time per seed, plus the risk
inherent in asking a model to do deterministic bookkeeping (invent flags,
skip the close, mis-count the tracker).

## Decision

Approval flows reviewer -> **closeout** (script node, `shape=parallelogram`,
`output_schema="routing"`), never directly back to the planner. The closeout
script:

1. Closes the in-progress seed (`sd list --status in_progress` is the
   authoritative claim source — no context plumbing; the tracker IS the
   state).
2. Checks the tracker and emits routing output: `Tracker empty` (exit) or
   `More seeds` (planner, fresh claim).

Consequences for roles:

- The planner NEVER closes seeds and NEVER acts on `approved` verdicts
  (stale approved verdicts visible after closeout are ignored by prompt
  contract; scripts cannot clear context keys).
- The planner always starts on a fresh claim: `changes_requested` re-claims
  remain its only verdict handling.
- `sd close` authority: closeout script and humans. Nobody else.

Failure semantics: a failed closeout exits the run; the seed stays open and
the next run re-enters the review cycle (an approved-but-unclosed seed is
harmlessly re-approvable).

## Also in this ADR: budget and effort calibration

- `preamble_budget_kb` 12 -> 24: reviewer context peaked at 1.9% of the 1M
  window while a 14 KB evidence capture got blob-ref'd (one tool round-trip
  per review). The window is not the constraint; the detour is.
- Reviewer `reasoning_effort=low` (measured: 3,424 reasoning tokens — more
  than planner or implementer — for checklist verification; the inline
  PASS/FAIL report and evidence capture carry the load). Re-measure next
  run; revert on quality regression.
- Implementer: quality gate explicitly forbidden (compile or one focused
  test only) — the tester owns the gate; triple-gating the same tree wastes
  cold-cache time and blurs roles.

## Consequences

- One LLM session and ~7% wall time saved per seed; deterministic close
  authority; routing on tracker state is structural (edge conditions), not
  model-read.
- Platform follow-ups (engine seeds): response.* payload deduplication,
  distinct terminal nodes for notification semantics, cycle guards as edge
  conditions, prompt-cache prefix stability.
