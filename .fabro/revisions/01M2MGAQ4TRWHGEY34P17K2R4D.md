# Revision — run 01M2MGAQ4TRWHGEY34P17K2R4D

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2MGAQ4TRWHGEY34P17K2R4D.md
- seeds filed: fabro-8c75 (deterministic merge-commit duplicate matching in implementer preflight); fabro-c00c (forbid implementer self-run introspection)
- basis: run 01M2MGAQ4TRWHGEY34P17K2R4D, workflow version aa6198ef448e0dcd20e78cbe6b42372671b9e7411022835afc3fb3029a4db9cb, commit caf28297
- revised_at_commit: caf28297 (ADR-0015: engine drift signal for later judgement)

## Findings

### Implementer preflight: match duplicates against merge commits only, not any commit
- filed: fabro-8c75 (priority 1)
- Change: in `.fabro/workflows/develop/prompts/implementer.md` step-1 preflight, replace the any-commit grep + LLM judgment (fabro-58cb, PR #161) with `git log --merges --oneline origin/<merge-target> --grep "<seed>"` or the `(#<n>)` squash-subject pattern. Effect: duplicate detection stops depending on model judgment; the false-`Blocked` class (matched bd41cbac revisor seed-filing commit on `origin/denkhaus`) stops firing on every revisor-filed seed.

### Forbid the implementer from introspecting its own run
- filed: fabro-c00c (priority 2)
- Change: one line in the Input section of `.fabro/workflows/develop/prompts/implementer.md`: never call `fabro_run_get`/`fabro_runs_list` on your own run. Effect: avoids the full run-projection context dump (event seq 99, 82k input tokens, 71% of run cost) and a likely extra reasoning round per implementer pass.
