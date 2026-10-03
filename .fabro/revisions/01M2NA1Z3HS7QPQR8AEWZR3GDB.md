# Revision — run 01M2NA1Z3HS7QPQR8AEWZR3GDB

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2NA1Z3HS7QPQR8AEWZR3GDB.md
- seeds filed: fabro-4c81 — Add a deterministic pre-claim path-resolution check for seed bodies
- basis: run 01M2NA1Z3HS7QPQR8AEWZR3GDB, workflow version e1bda2f455a07c0d501ab040c95e4344d05b2f486e5a471db6e4c56668ad1c61, commit 28afb67b0eecd44214fc91b5f8a622a6ec3dbc98
- revised_at_commit: 28afb67b0eecd44214fc91b5f8a622a6ec3dbc98 (ADR-0015: engine drift signal for later judgement)

## Findings

### Add a deterministic pre-claim path-resolution check for seed bodies — filed fabro-4c81

Concrete change: extend `.fabro/scripts/dup-run-check.nu` or add a sibling `claim-check.nu <id>` that resolves every repo path named in a seed body and requires (or prints) the corrected body before `sd update --status in_progress` is legal.

Expected effect: each stale-path seed gets corrected once in-tracker instead of being re-derived by every later planner pass. In this run, the planner diagnosed fabro-6a77's spec naming a nonexistent node but skipped planner.md step 3's mandated `sd update --description` correction (seq 115); the wrong path stayed frozen in the seed row and the wrong-path hunt drove most of the 143s/$0.185 planner burn. Closed fabro-c0a8 landed only the prompt-side mandate, which this run shows is skipped at reasoning_effort=low. Duplicate searches ("claim-check", "path-resolution") found no open or closed seed covering deterministic claim-time enforcement.
