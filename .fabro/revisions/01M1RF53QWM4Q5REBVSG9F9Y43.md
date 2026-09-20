# Revision — run 01M1RF53QWM4Q5REBVSG9F9Y43

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M1RF53QWM4Q5REBVSG9F9Y43.md
- seeds filed:
  - fabro-56c2 — State the fs_hide shell bypass plainly in develop prompts
  - fabro-7e88 — Fast-path `sd show <id>` in planner when the goal names a seed id
  - fabro-3839 — Lint filed seeds for resolvable basis refs and tool-name typos
  - fabro-a0e3 — Fix Pipeline progress header's unique-node denominator
- basis: run 01M1RF53QWM4Q5REBVSG9F9Y43, workflow version absent, commit 3ca43b1cd19c8cf291668bdbdc4a1fbbc321e2ec
- revised_at_commit: 3ca43b1cd19c8cf291668bdbdc4a1fbbc321e2ec (ADR-0015: engine drift signal for later judgement)

## Findings

### State the fs_hide shell bypass plainly in develop prompts
Filed: fabro-56c2 (priority 1)
Concrete change: in `.fabro/workflows/develop/prompts/implementer.md` (Platform scope section) and `prompts/planner.md` (ADR-0015 stale-basis step), replace the "writes are denied (fabro-1dae fs_hide)" claim with explicit guidance that for platform-targeting seeds, `.fabro/**` reads/writes go through the shell because fs_hide binds tool calls only. Expected effect: 3-4 tool calls and 2-3 LLM turns saved per platform-targeting seed; prompts stop asserting unenforced behavior.

### Fast-path `sd show <id>` in planner when the goal names a seed id
Filed: fabro-7e88 (priority 2)
Concrete change: add a rule to `.fabro/workflows/develop/prompts/planner.md` step 1 — if the goal names a seed id, run `sd show <id> --format json` first, fall back to `sd ready` only if unresolved. Expected effect: ~14 KB less context and one fewer call in the planner stage (63% of run cost).

### Lint filed seeds for resolvable basis refs and tool-name typos
Filed: fabro-3839 (priority 2)
Concrete change: in the revisor/improve seed-intake stage writing `.seeds/issues.jsonl`, require a resolvable run-id/Basis line per ADR-0015 and flag tool-name typos against the pinned-toolchain list. Expected effect: planner passes stop paying the contradiction-resolution premium on defective briefs (typo `sd`/`sed`, unresolvable `fabro-28e8`, conflicting call-site counts in `fabro-37a6`).

### Fix Pipeline progress header's unique-node denominator
Filed: fabro-a0e3 (priority 2)
Concrete change: fix the unique-node denominator in the stage-preamble "Pipeline progress" rendering so already-completed stages count. Expected effect: correct loop-state information in every stage preamble.
