# Revision — run 01M2VAAYWA0H7XN0V8WSRQ29FE

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2VAAYWA0H7XN0V8WSRQ29FE.md
- seeds filed: fabro-846b (lint bare slash-tokens in skills=discover prompts), fabro-e8ae (fix anchor-path truncation in planner-preflight.nu), fabro-d9f9 (PROJECT_FACTS: vendored crate sources absent in run containers)
- basis: run 01M2VAAYWA0H7XN0V8WSRQ29FE, workflow version 8dafcf4ebc5c1b1a4781f85b7f5c1a6125f166bfcdef6bbc34414c18e13aa5f3, commit 7cca0142f65d298d4cbaec86c1422def2b4d0ef1
- revised_at_commit: 7cca0142f65d298d4cbaec86c1422def2b4d0ef1 (ADR-0015: engine drift signal for later judgement)

## Findings

### Lint bare slash-tokens in prompts of skills=discover nodes
- filed: fabro-846b
- PR #246 (fabro-f84f) opted six stages into `skills="discover"`, re-enabling the bare `/name` expansion class that killed the conductor line (fabro-4dd8). Add a deterministic lint beside `lint-nu` in `.fabro/workflows/develop/scripts/qualitygate.nu` and in validate-workflows, scanning prompts and known context-rendered text of opted-in nodes for bare slash-tokens. Engine-side fixes exist (fabro-26c3, fabro-68d3, fabro-af22) but no seed provides a fabro-side interim guard.

### One-character anchor-path truncation in planner-preflight.nu
- filed: fabro-e8ae
- Preflight flagged fabro-c643 with `.fabro/Dockerfile.toolchai` (truncated by one char vs the seed body). Fix anchor-path extraction in `.fabro/workflows/develop/scripts/planner-preflight.nu` so verdict-table paths match seed bodies exactly. Distinct from fabro-7611 (crate-relative resolution) and fabro-7daf (content checks).

### PROJECT_FACTS: vendored crate sources absent in run containers
- filed: fabro-d9f9
- Planner wasted three probe calls on `~/.cargo/git/checkouts` plus a 1.6s `find /` hunting the vendored pebble crate. Add one PROJECT_FACTS bullet: judge engine/dependency behavior from the workspace tree only; cargo checkouts are absent in run containers. Saves 2–3 dead-end calls per engine-behavior seed.
