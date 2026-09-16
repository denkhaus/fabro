## FACTS — the loop and repo values this architect runs on

This block is the ONE place the architect prompts carry facts about THIS
repository and its loop tooling (ADR-0013 pattern; the stage prompts stay
project-agnostic). Porting the loop means editing this file plus the
workflow graph, not the prompts. A stale value here is loop friction:
report it in the journal, never silently work around it.

- Issue tracker — the `sd` CLI (Seeds, git-native in `.seeds/`). Seed ids
  carry the prefix `fabro-` (e.g. `fabro-37a6`); the supported read path
  is `sd show <id> --format json`. `sd search` matches title/description
  text only and is AND-strict: use ONE keyword per query (broaden by
  dropping words); `sd show`, never `sd search`, for id lookups.
- Friction score — `nu .fabro/scripts/friction-score.nu` computes the
  deterministic 0.0-1.0 systemic-friction score from tracker + journal
  metrics. Output: ONE JSON object with `score`, `verdict`
  (normal | grind | architecture-due), `threshold`, `components`,
  `inputs`. Exit 0 ALWAYS — a score is a measurement, not a pass/fail.
- Architecture artifacts — `.fabro/architecture/`:
  `reviews/<YYYY-MM-DD>.md` (fabro_ask answer, verbatim) and
  `<YYYY-MM-DD>-pass.md` (cooldown marker written by the file stage; its
  presence within 48h parks the survey).
- Stage journal — `.fabro/journal/<run_id>.jsonl`: one JSON record per
  stage completion. A journal whose lines contain `"node":"planner"`
  belongs to a develop run (the develop-workflow signature); the freshest
  such journal names the freshest develop run id (filename stem).
- Decision records — ADRs live in `docs/lab/adr/`; strategy docs
  (logging, events, testing, secrets, migrations, error handling) live in
  `docs/internal/`. Read them via shell (they are fs_hide-bound for FILE
  tools in this flow).
- Merge-target branch — `origin/denkhaus`: the branch this line's run
  PRs integrate into.
- Engine-provided credentials (ADR-0019 review axis): the engine injects
  `GITHUB_TOKEN` into every agent shell call (`resolve_workflow_env` in
  `lib/components/fabro-workflow/src/services.rs`) and runs a git
  credential bridge (`lib/components/fabro-workflow/src/git_bridge.rs`) —
  agent-reachable capability that is invisible from the container
  environment alone. Architecture findings that would add, change, or
  remove such capability file as `needs-user` seeds.
