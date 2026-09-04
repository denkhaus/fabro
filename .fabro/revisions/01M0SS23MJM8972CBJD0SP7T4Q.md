# Revision — run 01M0SS23MJM8972CBJD0SP7T4Q

- status reviewed: failed
- review: .fabro/reviews/develop/01M0SS23MJM8972CBJD0SP7T4Q.md
- seeds filed: fabro-6bbe Clear review_feedback in the reviewer Approved JSON template; fabro-ff7a Route no-change verification resubmits straight to reviewer; fabro-8d61 Document glob_filter glob syntax in the planner prompt

## Findings

### Clear review_feedback in the reviewer Approved JSON template — filed fabro-6bbe
Add `"review_feedback": ""` to the Approved JSON template in `.fabro/workflows/develop/prompts/reviewer.md` so a verdict flip resets stale rejection text. planner@3 carried cycle-1 'changes requested' feedback alongside an approved verdict, leaking dead instructions into the next re-plan. Complements fabro-5870; not a duplicate (different trigger point: verdict flip vs closeout).

### Route no-change verification resubmits straight to reviewer — filed fabro-ff7a
Add an `Implemented (no changes)` label edge from implementer to reviewer in `.fabro/workflows/develop/workflow.fabro` (or condition the tester edge on a dirty worktree). tester@2 and evidence@2 re-ran on a byte-identical worktree; the unconditional implementer→tester edge wastes a gate plus evidence pair per procedural resubmit.

### Document glob_filter glob syntax in the planner prompt — filed fabro-8d61
Note in `.fabro/workflows/develop/prompts/planner.md` that grep's `glob_filter` expects a glob (`*.go`), not a bare filename. planner@1 burned two empty grep calls plus a recovery find/head on `glob_filter: "main.go"`.
