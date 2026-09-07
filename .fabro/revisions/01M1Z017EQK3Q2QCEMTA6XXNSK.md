# Revision — run 01M1Z017EQK3Q2QCEMTA6XXNSK

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M1Z017EQK3Q2QCEMTA6XXNSK.md
- seeds filed: fabro-cf76 (planner verifies spec-named headings/anchors/paths), fabro-eed2 (document == evidence complete == marker in reviewer blob-ref paragraph)
- basis: run 01M1Z017EQK3Q2QCEMTA6XXNSK, workflow version 78ff76ac6dc73632d1cb65e7e2667edcf4d8b52bf90c47667776f8195ee52e5f, commit 91e7b67f7ec6a4f9470dc082882f4cd9fc59b59f
- revised_at_commit: 91e7b67f7ec6a4f9470dc082882f4cd9fc59b59f (ADR-0015: engine drift signal for later judgement)

## Findings

### Planner: verify spec-named headings/anchors/paths exist in target files before forwarding the brief
- filed: fabro-cf76 (priority 1)
- In `.fabro/workflows/develop/prompts/planner.md` step 7 (contradiction check): when the spec names a heading/anchor/path, confirm it exists in the target file; otherwise annotate the actual location in the brief. Evidence: brief forwarded "checklist gains a CAPABILITY DELTA axis" verbatim, but `reviewer.md` has no "checklist" heading (the numbered "Your job this pass" list is it); the implementer burned ~14 s / 519 reasoning tokens deciding placement (events seq 95-97). Expected effect: shorter implementer passes, fewer misplacement deviations → fewer Changes-requested cycles (~2.5 min + ~$0.15 each).

### Reviewer prompt: document the `== evidence complete ==` terminal marker in the blob-ref paragraph
- filed: fabro-eed2 (priority 2)
- In `.fabro/workflows/develop/prompts/reviewer.md` (blob-ref paragraph), add one sentence: the capture ends with `== evidence complete ==` (printed by `scripts/evidence.nu:431`); seeing it means the diff was not cut. Evidence: this run's reviewer journaled the marker "was the deciding signal" for completeness but only found it after paging to the end of the blob. Expected effect: reviewers approve from a single confident paged read (~40 s saved), complements fabro-a0fe and fabro-8d2c without overlap.
