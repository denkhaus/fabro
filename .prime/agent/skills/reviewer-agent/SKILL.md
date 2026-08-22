---
name: reviewer-agent
description: Static code review for Fabro workflow repos. Scans .fabro/workflows (especially the develop workflow) and repo wiring for weaknesses, gaps, and project-agnosticity violations - leaked project names, seed-id prefixes, repo file references, and tool references outside the allowed set (just, ml, sd). Use when asked to review fabro workflows, check project-agnosticism of workflow assets, audit the develop workflow, or find gaps against docs/fabro documentation.
---

# reviewer-agent

Reviews a repository that hosts Fabro workflows. Every finding carries a rule
id, severity, and exact `path:line` references, plus a concrete suggestion.

## Call from the kernel

    await reviewer_agent(root="/path/to/repo", workflow="develop")

Or from a shell cell:

    !reviewer_agent --root /path/to/repo --workflow develop

## What it checks

1. **Agnosticity** — the workflow and all its assets (graph, prompts,
   scripts, workflow.toml) must be project-agnostic:
   - no references to the current project's name, seed-id prefix, or files
   - no project tooling referenced except `just`, `ml`, `sd`
   - `git`/`fabro` count as platform substrate (Fabro checkpoints are plain
     Git; docs/fabro/checkpoints.md) and stay allowed
2. **Graph correctness** against the local docs in `docs/fabro/` — outcome
   values, prompt/edge label-routing contract, failure-swallowing exit edges,
   missing safeguards (timeout, max_node_visits, goal_gate), dangling refs,
   template variables, reachability.
3. **Scripts** — checkpoint-base detection fragility, untracked-artifact blind
   spot of the gate bridge, existence of referenced `just` recipes,
   diff-filter gaps (e.g. deletions missing from per-file diffs).
4. **Prompts** — cross-node context-key flow, goal-injection guards, reviewer
   ground truth, planner shortcuts that bypass gate/review.
5. **Repo wiring** — project.toml prepare steps, provisioning of the tool
   contract.

Full rule catalog with severity and docs citations:
[references/rules.md](references/rules.md).

## Parameters

| Parameter | Default | Meaning |
|---|---|---|
| `root` | `.` | Repository root to scan |
| `workflow` | `"develop"` | Workflow name under `.fabro/workflows/`; `None` = all |
| `allowed_tools` | `("just","ml","sd")` | Tools the workflow may reference |
| `platform_tools` | `("git","fabro")` | Always-allowed platform substrate |
| `min_severity` | `"info"` | `error`, `warn`, `info`, or `pass` |
| `format` | `"markdown"` | `markdown` report or `json` findings |
| `report_dir` | `"docs/reviews"` | Directory (relative to `root`) where markdown reports are saved as `<workflow>-review-<shortsha>.md`. Each report is dated: a visible `> Reviewed: **YYYY-MM-DD** - branch@sha` line under the title plus a machine-readable `reviewed commit:` comment. `None` disables saving; `json` format never saves. |

## Notes

- The project fingerprint is auto-detected (`.seeds/config.yaml` project name,
  go.mod / package.json / Cargo.toml names, `.mise.toml` tools, `git ls-files`,
  justfile recipes). No configuration needed.
- The word "fabro" is treated as platform vocabulary (`.fabro/` paths,
  `Fabro-` trailers, `fabro(` commit subjects) unless it appears as a project
  or seed identifier.
- Pure stdlib; runs offline. Docs citations reference the local documentation
  snapshot in `docs/fabro/` of the scanned repo.
- Reviews are platform assets: they live under `docs/reviews/` on the
  platform world's repo, even when the scanned root is the product world —
  pass `scanned_root` to `save_report` so the header names the scanned
  product commit. The returned markdown ends with a `Saved to:` footer.
