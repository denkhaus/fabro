# Revision — run 01M0QJ01P9E6EKX8S6TQN91G8E

- status reviewed: failed
- review: .fabro/reviews/develop/01M0QJ01P9E6EKX8S6TQN91G8E.md
- seeds filed: fabro-7bea — Add repair-and-resume from checkpoint for deterministic config failures

## Findings

### Add repair-and-resume from checkpoint for deterministic config failures
- filed id: fabro-7bea
- The planner stage died on its first LLM call (invalid `reasoning_effort` pin, root cause already tracked as `fabro-b158`) and the run went terminal with "goal gate unsatisfied for node planner and no retry target" despite event seq 43 checkpointing the failed state. Proposed engine/graph change: allow a terminally failed run to resume from its checkpoint after the underlying config is fixed, or add a hold node before terminal exit. Expected effect: a 10-second human fix continues the run at node cost instead of forcing a fresh run with another ~33s sandbox/clone/mise/bootstrap setup. Distinct from closed `fabro-18a5` (terminal classification) and open `fabro-53d3` (Blocked edge); cross-referenced with `fabro-b440` (fork-from-checkpoint ends terminal failed) as a complementary case, not a duplicate or supersession.
