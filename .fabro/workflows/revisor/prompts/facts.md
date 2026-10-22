## FACTS — the loop and repo values this revisor runs on

This block is the ONE place the revisor prompts carry facts about THIS
repository and its loop tooling (ADR-0013 pattern; the stage prompts stay
project-agnostic). Porting the loop means editing this file plus the
workflow graph, not the prompts. A stale value here is loop friction:
report it in the journal, never silently work around it.

- Work-item tracker (ADR-0032) — the project's work items live in the
  central tracker behind the fabro server; agent access is EXCLUSIVELY
  the engine-mediated run tools: `fabro_work_show` (one item by key),
  `fabro_work_list` (open items; with `query` a plain-keyword text
  search over title AND body, INCLUDING closed items — the duplicate
  check before filing), `fabro_work_create`, `fabro_work_close`,
  `fabro_work_release`. Item keys are provider-stable and look like
  `denkhaus/work#123` — copy them verbatim, never truncated. No tracker
  CLI, no tracker file, no forge client: everything goes through the
  tools (ADR-0019).
- Merge-target branch — `origin/denkhaus`: the branch the line's run PRs
  integrate into. Duplicate-run adjudication checks exactly this branch:
  a landed implementation commit whose run trailer names the target run
  is a self-closure (healthy); a closure by any other run makes the
  target a duplicate.
- Stage journal — `.fabro/journal/<run_id>.jsonl`: the fallback source
  for the work item a reviewed run claimed (the claim/grant record names
  the item key; shell grep for the `#`-key shape when the run's goal
  names none).
- Engine-provided credentials (ADR-0019 review axis): the engine injects
  `GITHUB_TOKEN` into every agent shell call (`resolve_workflow_env` in
  `lib/components/fabro-workflow/src/services.rs`) and runs a git
  credential bridge (`lib/components/fabro-workflow/src/git_bridge.rs`) —
  agent-reachable capability that is invisible from the container
  environment alone.
