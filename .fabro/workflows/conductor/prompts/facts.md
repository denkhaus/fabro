## FACTS — the repo-specific values this conductor runs on

This block is the ONE place the conductor prompts carry facts about THIS
repository (ADR-0013 pattern; the leg prompts stay project-agnostic).
Porting the loop to another project means editing this file plus the
workflow graph, not the prompts. A stale value here is loop friction:
report it in the journal, never silently work around it.

- Merge-target branch — the branch this line's run PRs integrate into:
  `origin/denkhaus`. The upstream-drift count the survey computes is
  `git rev-list --count origin/denkhaus..upstream/main`.
- Upstream mirror — remote `upstream` -> `https://github.com/fabro-sh/fabro`
  (branch `main`). `origin/main` is only the upstream fabro-sh mirror and
  never carries the line's merges; branch-sensitive checks name the
  merge-target branch above, never the mirror.
- Child target for EVERY `fabro_run_create` — ALWAYS EXPLICIT:
  `{"kind": "git", "repo": "denkhaus/fabro", "branch": "denkhaus"}`.
  Omitted targets inherit the parent's RUN BRANCH (a `fabro/run/…` ref
  where branch protection, the Dogfood Gate, and auto-merge do not exist).
- Stage journal — `.fabro/journal/<run_id>.jsonl`: one JSON line per stage
  completion; the seed id a run claimed is recoverable by grepping it for
  the tracker's seed-id prefix.
- Revision markers — `.fabro/revisions/<run-id>.md`: a develop or
  merge-upstream run without a marker on the base branch is unrevised.
