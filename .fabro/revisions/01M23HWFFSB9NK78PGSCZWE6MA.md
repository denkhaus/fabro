# Revision — run 01M23HWFFSB9NK78PGSCZWE6MA

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M23HWFFSB9NK78PGSCZWE6MA.md
- seeds filed: fabro-6b58 (bound planner in-flight check with created_since + self-exclusion), fabro-8d4c (probe file structure with heading greps, never head -N), fabro-7808 (document grep exit 1 as expected no-match in implementer verification)
- basis: run 01M23HWFFSB9NK78PGSCZWE6MA, workflow version 70cee3b32d01c25022b0c84ba4b42f2e8c93d679f400542db4ca1cafafa1a6c1, commit 515758e102a81c1c35d8c41bb3fd96209f1f04e9
- revised_at_commit: 515758e102a81c1c35d8c41bb3fd96209f1f04e9 (ADR-0015: engine drift signal for later judgement)

## Findings

### Bound the planner in-flight check: created_since window plus explicit self-exclusion
- filed: fabro-6b58
- In `.fabro/workflows/develop/prompts/planner.md` step 4 (IN-FLIGHT PR CHECK): call `fabro_runs_list` with `created_since` ≈ 48 h and exclude this run's own id from the in-flight set. Expected effect: tool output drops ~14.5 KB to ≤2 KB per run; the self-run confusion class disappears. Regression of closed seed fabro-9967.

### Planner prompt: probe file structure with heading greps, never head -N full files
- filed: fabro-8d4c
- In `.fabro/workflows/develop/prompts/planner.md` step 3 (stale-basis verification): probe structure with `grep -n "^#" <file>` and `ls <dir>`, never `head -N` full files. Expected effect: ~4 KB less context and fewer probe calls per run. Distinct from open fabro-9fa2 (different trigger condition).

### Implementer prompt: state that grep exit 1 on pre-change verification is an expected no-match
- filed: fabro-7808
- In `.fabro/workflows/develop/prompts/implementer.md` verification section: add 'grep exit 1 = no match; on pre-change verification that is the expected state, not an error.' Expected effect: removes a recurring false-error signal; orthogonal to open fabro-a23f.
