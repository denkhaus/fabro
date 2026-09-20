# Revision — run 01M22X87J1RKQ6A8FZB7RC2RN8

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M22X87J1RKQ6A8FZB7RC2RN8.md
- seeds filed: fabro-2eb6 (implementer prompt: rg replace-flag discipline line; retract the false 'sandbox rg unreliable' journal lore), fabro-9372 (expose current_seed_id in the fabro_runs_list projection — stop the planner reverse-engineering the seed id), fabro-7613 (engine: auto-stamp a painpoint when a stage pages through a preamble blob ref its journal never reported as friction)
- basis: run 01M22X87J1RKQ6A8FZB7RC2RN8, workflow version 14ab451ff2f9375f4927ce7d994c63bc96f48f3c617f3baff15119472f3af3e4, commit 757bd457f0405a9889aa62144ea0c3f60581e655
- revised_at_commit: 757bd457f0405a9889aa62144ea0c3f60581e655 (ADR-0015: engine drift signal for later judgement)

## Findings

### Implementer prompt: rg replace-flag discipline line; retract the false 'sandbox rg unreliable' journal lore

- filed: fabro-2eb6 (priority 1)
- change: add one line to `.fabro/workflows/develop/prompts/implementer.md`: `rg -r <text>` REPLACES matches — never write `rg -rn`; `-n` alone is the line-number flag.
- expected effect: eliminates the `rg -rn` error class (rg parses `-r n` as replace-with-literal-n) and stops the durable false 'sandbox rg unreliable' platform lore that cost 2 extra diagnostic rounds in this run.

### Expose current_seed_id in the fabro_runs_list projection — stop the planner reverse-engineering the seed id

- filed: fabro-9372 (priority 2)
- change: engine-side, extend the `fabro_runs_list` run projection with the run's `current_seed_id`; interim one-liner in `.fabro/workflows/develop/prompts/planner.md` step 4 to read it from `.fabro/journal/<run_id>.jsonl`.
- expected effect: −3 tool calls and −2 LLM rounds per planner pass whenever any PR is open; removes a fragile journal-grep heuristic that could mis-skip a valid candidate or miss a real double-pick.

### Engine: auto-stamp a painpoint when a stage pages through a preamble blob ref its journal never reported as friction

- filed: fabro-7613 (priority 2)
- change: in fabro-workflow, detect blob-path reads (`/tmp/fabro/runtime/blobs/...`) in the stage tool log and either append a deterministic painpoint stub to the stage journal or lint the blob-read-with-empty-painpoints mismatch at stage completion (reusing the ContextKeyOmitted lint machinery).
- expected effect: evidence-pipe friction reaches the improve loop deterministically instead of via self-report, which demonstrably failed here (`painpoints: []` while the reviewer paged through a demoted evidence capture).
