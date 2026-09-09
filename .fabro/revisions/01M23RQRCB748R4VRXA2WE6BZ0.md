# Revision — run 01M23RQRCB748R4VRXA2WE6BZ0

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M23RQRCB748R4VRXA2WE6BZ0.md
- seeds filed: fabro-5656 Implementer base-branch merged-seed check; fabro-b073 Planner re-plan audits run-base diff; fabro-e19f Engine rollback/side-ref on deterministic failure; fabro-5e62 Evidence capture empty-diff hard error + intersecting loop-work diff; fabro-574d Qualitygate per-seed touched crates; fabro-d898 Reviewer accounts for every run-base-diff file; fabro-11d3 Planner tie-break prefers failure-class prevention; fabro-55a7 Planner batches reconnaissance into one shell call
- basis: run 01M23RQRCB748R4VRXA2WE6BZ0, workflow version 3b17faf145735aa14caf619a38c0b079a1ff1c91acf24211c9ab08a2612c6f6f, commit d55d6ea53c41f53c116ba063221f2988223072dd
- revised_at_commit: d55d6ea53c41f53c116ba063221f2988223072dd (ADR-0015: engine drift signal for later judgement)

## Findings

### Implementer: check the base branch for an already-merged seed before any edit
- filed: fabro-5656 (priority 1)
- Add a step-1 pre-edit check in `implementer.md`: `git fetch origin <base> && git log --oneline origin/<base> --grep "<seed id>"`; a hit routes Blocked as duplicate. Prevents a repeat of the fabro-6a5a duplicate burn (731 s / $0.37, 52% of run cost).

### Planner re-plan: audit the run-base diff for failed-stage leftovers, never git status
- filed: fabro-b073 (priority 1)
- Re-plan must audit `git diff <run-base>..HEAD --stat` and revert leftovers; clean `git status` missed commit `ae95484` and PR #109 re-shipped 85 lines of user-reverted `auto_merge = false`.

### Engine: roll back or side-ref the worktree diff when a stage fails deterministically
- filed: fabro-e19f (priority 2)
- Checkpoint path (`lib/components/fabro-workflow/src/pipeline/`) should roll back or side-ref the worktree diff on `failure_class=deterministic` so failed-stage commits never land on the run branch.

### Evidence capture: hard-error on seed-work file with empty diff body; render loop-work diff on target intersection
- filed: fabro-5e62 (priority 2)
- `evidence.nu` hard-errors on empty seed-work diff bodies and renders loop-work diff when churn intersects seed targets. Complements closed fabro-4b57; claim-base resolution tracked in fabro-1e9f (cross-referenced, not superseded — different mechanism).

### Qualitygate: derive touched crates from the per-seed base so Markdown-only seeds skip the Rust gate
- filed: fabro-574d (priority 2)
- Apply the per-seed claim-base fix (fabro-1e9f, cross-referenced) to touched-crate detection; basis run paid a 23.5 s full-crate gate for a 2-line Markdown seed.

### Reviewer: account for every file in the run-base diff; sub-claim-base changes are deviations
- filed: fabro-d898 (priority 2)
- Reviewer prompt step 2 must reconcile every file in `git diff <run-base>..HEAD --stat`; the basis run approved PR #109 whose diff vs run base contained the workflow.toml trio plus `pull_request.rs`.

### Planner tie-break: prefer seeds closing an observed failure class over same-priority polish
- filed: fabro-11d3 (priority 2)
- Tie-break line in planner step 2 preferring prevention seeds (fabro-9372, fabro-6b58) over same-priority polish; basis run claimed polish while the duplicate-claim race recurred (12 min, manual halt).

### Planner: batch reconnaissance into one shell call (sd ready + sd show + base-branch grep)
- filed: fabro-55a7 (priority 2)
- Single shell call for reconnaissance; ~40–60 s and ~$0.06 saved per planner pass (36% of run cost) and a narrower claim-to-dispatch window.

No supersessions; no capability-affecting findings (all fixes are prompt/script/engine-internal, no credential or tool deltas).
