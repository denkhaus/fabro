# Revision — run 01M0WWKAQCWZC0Q0JK019H0ZC7

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M0WWKAQCWZC0Q0JK019H0ZC7.md
- seeds filed: fabro-08dd (Bump evidence diff context from -U1 to -U3), fabro-cf03 (Make engine checkpoint metadata snapshots async or branch-point-only), fabro-aa3d (Legalize sd list for goal-condition open-seed verification in planner prompt)

## Findings

### Bump evidence diff context from -U1 to -U3
- filed: fabro-08dd
- Change `git diff -U1` to `-U3` in `.fabro/workflows/develop/scripts/evidence.nu`. The reviewer journal in this run admits trusting the green gate and pinning tests rather than re-deriving range math, because -U1 omitted where start/last are computed in `run()` — the exact code under review. Distinct from fabro-3c9d (sort+budget), fabro-1e9f (diff base), fabro-4b57 (loop-work diff). Expected effect: range/validation logic verifiable in-diff; approvals-by-trust eliminated.

### Make engine checkpoint metadata snapshots async or branch-point-only
- filed: fabro-cf03
- Make the metadata snapshot fire-and-forget async, or run only on branch/failure nodes (tester/evidence/reviewer exits). Every checkpoint in this run paid a synchronous ~2.5s snapshot (~17s total, 6% of 293s wall) on a linear zero-retry path nothing resumes from. Complements (does not supersede) open fabro-c2ca, which is a config-level skip for non-agent stages only — the planner snapshot here is an agent stage and would remain. Expected effect: ~15-17s shaved per seed cycle.

### Legalize sd list for goal-condition open-seed verification in planner prompt
- filed: fabro-aa3d
- Reword the sd table in `.fabro/workflows/develop/prompts/planner.md`: `sd list` is expected when the goal requires confirming no other open seed remains (blocked-but-open seeds are invisible to `sd ready`). The planner here correctly ran `sd list --format json` (seq 68) despite the prompt's prohibition — a must-be-disobeyed instruction. Reconcile with fabro-e4fa/fabro-b382 (thematic overlap, not supersession): b382's "sd ready empty → Tracker empty, do not run sd list" would misroute while blocked-but-open seeds remain. Expected effect: no must-be-disobeyed instruction; wrongful closeout routing guarded.
