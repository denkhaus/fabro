# Revision — run 01M299PVMSE7Q53ZGPP67AHNMG

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M299PVMSE7Q53ZGPP67AHNMG.md
- seeds filed: fabro-7e8d (quarantine flaky e2e_stall_watchdog_with_explicit_timeout_override), fabro-020b (cap loop-churn diffs in evidence.nu), fabro-1a35 (PROJECT_FACTS crate path resolution via symbol search)
- basis: run 01M299PVMSE7Q53ZGPP67AHNMG, workflow version e825d8c827ce15defe0aa57d3586b0f369cab48cc7fe711d526776762f442aa1, commit 6f628c21a79469837da6b9bef63903e5f40c1f65
- revised_at_commit: 6f628c21a79469837da6b9bef63903e5f40c1f65 (ADR-0015: engine drift signal for later judgement)

## Findings

- Quarantine the known-flaky e2e_stall_watchdog_with_explicit_timeout_override test — filed fabro-7e8d. Mark the test flaky (#[ignore] or nextest retries override) and file the tracking seed; drops implementer verification to one suite run and removes a ~22-min gate-red tail risk. No duplicates (checked fabro-56f4 closed, fabro-80b3 different test).
- Cap loop-churn diffs in evidence.nu to numstat plus first ~30 changed lines — filed fabro-020b. Keeps 94KB evidence captures under the reviewer's 16KB inline ceiling, eliminating the per-cycle blob-paging detour. Cross-references open fabro-3c9d/fabro-8d2c/fabro-9837 as complementary budget-side work.
- PROJECT_FACTS: resolve abbreviated crate paths by symbol search — filed fabro-1a35. One-line prompt addition stopping planner probes of non-existent lib/foundation/fabro-llm paths; ~30-45s saved per claim. Excludes fabro-0586's fd-not-installed scope.
