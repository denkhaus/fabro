# Revision — run 01M2GVW7GGGB19ZZ9NBN2H5FDC

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2GVW7GGGB19ZZ9NBN2H5FDC.md
- seeds filed: fabro-3f62 (downstream-caller focused tests on cross-crate contract changes), fabro-b798 (per-file diff-stat map at top of evidence.nu output), fabro-8abd (planner salvage pointers cite stranded commit + `git show` form), fabro-a643 (extra focused tests must name their acceptance bullet)
- basis: run 01M2GVW7GGGB19ZZ9NBN2H5FDC, workflow version 4fd3342d2e46ca36d4b0eb600249e5fe5efd7c268e3a0135acc98509a627a903, commit 5f154a8bf642b47ed7311b5c58c2166024183e89
- revised_at_commit: 5f154a8bf642b47ed7311b5c58c2166024183e89 (ADR-0015: engine drift signal for later judgement)

## Findings

### Implementer verification policy: run downstream-caller focused tests on cross-crate contract changes
- filed: fabro-3f62
- change: implementer.md step 4 rule — rg the workspace for callers/tests of a changed cross-crate contract symbol and run those focused tests too
- effect: turns the observed ~4-minute gate-red diagnosis into a seconds-long focused failure; complements fabro-fc1b (closed) and fabro-6e7f (in progress)

### Evidence capture: emit a per-file diff-stat map at the top of evidence.nu output
- filed: fabro-b798
- change: prepend a compact per-file plus/minus stat map ahead of the full diff in evidence.nu
- effect: fewer reviewer tool turns and less verification uncertainty per review; orthogonal to fabro-020b, fabro-3c9d, fabro-9837

### Planner salvage pointers must cite the stranded commit and `git show <commit>` form
- filed: fabro-8abd
- change: planner.md step 6 — stranded-branch seeds cite the implementer commit, not the branch tip
- effect: removes the mis-based diff call on every salvage pass (228 files of divergence observed before the correct `git show da3eaabb3` base)

### Focused tests beyond the touched-crate classes must name their acceptance bullet
- filed: fabro-a643
- change: implementer.md step 4 — extra focused tests must name the acceptance bullet they serve
- effect: ~5 min saved per implementer pass, redirected to tests that catch drift; distinct from fabro-ad1b (report-side citation)
