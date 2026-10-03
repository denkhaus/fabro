# Revision — run 01M2N1G834RTCSZNX7EK40G3SE

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2N1G834RTCSZNX7EK40G3SE.md
- seeds filed:
  - fabro-9d26 — First-pass verification-only fast path: planner -> evidence -> reviewer (P1)
  - fabro-f759 — Evidence capture: emit per-criterion check outputs for verification-only runs (P2)
  - fabro-5bbd — Planner prompt: require labeled segments in compound probe commands (P2)
  - fabro-9e8b — Re-verify the pipeline-progress header fix: post-#175 counter-evidence of miscounts (P2)
- basis: run 01M2N1G834RTCSZNX7EK40G3SE, workflow version 6ad783be021e539bec3ef20d5ab0773111d20471eab6b0892bc38e48239d1d19, commit 755dcbc17c443d377f0ef7d61640872df9c377b4
- revised_at_commit: 755dcbc17c443d377f0ef7d61640872df9c377b4 (ADR-0015: engine drift signal for later judgement)

## Findings

### First-pass verification-only fast path (fabro-9d26)
Add a planner->evidence edge (label 'Verification-only', condition on preferred_label) in `.fabro/workflows/develop/workflow.fabro` plus a matching output variant in `.fabro/workflows/develop/prompts/planner.md`. Run verified the same three fabro-3c9d criteria three times (planner pre-claim, implementer: 5 shell calls / 57 s / $0.064, reviewer: 3 shell calls), all green first-pass. Expected ~-60 s and ~-$0.07 (~30% of cost) per verification-only run. Distinct from open fabro-ff7a (resubmit-only edge).

### Per-criterion check outputs in verification-only captures (fabro-f759)
`.fabro/workflows/develop/scripts/evidence.nu`: when seed-work is empty and the brief is verification-only, emit the implementer's per-criterion check commands plus outputs. This run's capture was 1,774 bytes of '(no seed-work files to diff)', forcing the reviewer to re-derive criteria — the anti-pattern closed fabro-50c9 banned for diffs. Expected reviewer shell calls 3->0, ~12 s faster review. Complements fabro-b8ed; distinct from diff-carrying seeds fabro-750c/fabro-b798/fabro-8cef.

### Labeled segments in compound probe commands (fabro-5bbd)
One line in `.fabro/workflows/develop/prompts/planner.md`: when chaining probes, prefix each with `echo '=== <label> ==='`. Run seq 55-72: planner misattributed a chained `git log` output and burned 3 extra calls (~30 s) on follow-up archaeology. Prevents this ~30 s misdiagnosis class. Distinct from fabro-b6f9/fabro-8d4c.

### Pipeline-progress regression verification (fabro-9e8b)
Post-#175 (base d9ee233) counter-evidence: implementer read '0 of 7' after planner completed, reviewer read '2 of 7' with 4 non-meta nodes done, squash trailer said Fabro-Completed: 2 on a 6-stage run. Closed fabro-45bf is post-fix counter-evidence, so a regression-verification seed rather than a blind reopen.
