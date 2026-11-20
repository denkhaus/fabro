# ADR-0049: Loop assets are product surfaces — the develop line works them

Date: 2026-11-20. Status: accepted (operator decision, sprint 94).

## Context

The develop line's implementer could not touch loop assets
(`.fabro/**`, `scripts/**`, `justfile` — the fs_hide envelope) and the
lane-scope gates refused such diffs. The cost grew past its value:
develop runs failed whenever an item's fix needed a loop-side edit
(work#1450 and work#1523 had to be built session-side; PR #638 was
hand-built; the work#1166 reviewer-friction class), and the session
became a permanent loop-lane substitute — the line's self-healing was
switched off at exactly the surfaces that break most.

## Decision

Lift the separation. Loop assets are product surfaces:

- The develop implementer's `fs_hide` narrows to `.agents/**` (the
  sync-style-guide mirror; owned by `just sync-style-guide`, never
  hand edits). The planner narrows to the same list.
- The lane-scope gates (qualitygate `product-denied?`, the claim-gate
  surface extraction, the planner-preflight legends) deny only
  `.agents/` on product lanes.
- The prompts drop the boundary teaching and its carve-outs.

The existing chain is the guard: reviewer (reads every diff) → PR →
adjudicator verdict → merger smoke. A run edits its checkout, never
the graph it runs on — an edit binds only in subsequent runs
(admission re-checkout), so there is no same-run self-modification.

This supersedes the loop/develop fs-envelope separation (ADR-0009's
envelope posture for these paths) and narrows ADR-0019's capability
gate for this surface: the recorded operator decision IS the approval
ADR-0019 requires. The `lane:loop` label survives for bookkeeping;
loop-asset items are develop-claimable.

## Consequences

- A broken loop-asset edit can now reach the trunk through the
  reviewer/adjudicator chain. The containment is unchanged: the
  collector validates at admission, a red graph fails loudly at run
  create, and the next session repairs it — visible, not silent.
- The iterate skill's local-first carve-out and AGENTS.md's
  "unlandable by the develop line" rule are retired in the same
  change.
