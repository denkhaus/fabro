## FACTS — the loop and repo values this revisor runs on

This block is the ONE place the revisor prompts carry facts about THIS
repository and its loop tooling (ADR-0013 pattern; the stage prompts stay
project-agnostic). Porting the loop means editing this file plus the
workflow graph, not the prompts. A stale value here is loop friction:
report it in the journal, never silently work around it.

- Issue tracker — the `sd` CLI (Seeds, git-native in `.seeds/`). Seed ids
  carry the prefix `fabro-` (e.g. `fabro-37a6`); the supported read path
  is `sd show <id> --format json`. `sd search` matches title/description
  text only and is AND-strict: use ONE keyword per query (broaden by
  dropping words), and use `sd show` — never `sd search` — for id
  lookups.
- Merge-target branch — `origin/denkhaus`: the branch the line's run PRs
  integrate into. The duplicate-run preflight
  (`nu .fabro/scripts/dup-run-check.nu <seed-id>`) greps exactly this
  branch; it defaults to it.
- Stage journal — `.fabro/journal/<run_id>.jsonl`: the fallback source for
  the seed id a reviewed run claimed (shell grep for the `fabro-` prefix
  when the run's goal names none).
- Engine-provided credentials (ADR-0019 review axis): the engine injects
  `GITHUB_TOKEN` into every agent shell call (`resolve_workflow_env` in
  `lib/components/fabro-workflow/src/services.rs`) and runs a git
  credential bridge (`lib/components/fabro-workflow/src/git_bridge.rs`) —
  agent-reachable capability that is invisible from the container
  environment alone.
