## PROJECT_FACTS — the repo-specific values this workflow runs on

This block is the ONE place the develop workflow carries facts about THIS
repository (ADR-0013; the prompts that include this block stay
project-agnostic). Porting the workflow
to another project means editing this file plus the workflow graph and
settings, not the prompts. A stale value here is loop friction: report it
in the journal, never silently work around it.

- Primary code areas — where item work normally lands: the Rust workspace
  under `lib/` (crates in `lib/apps/`, `lib/components/`, `lib/foundation/`),
  rarely `apps/` for the web UI.
- Loop-asset paths — the dev loop's own machinery, hidden from FILE TOOLS by
  the per-node `fs_hide` envelope (ADR-0009 stage-envelope
  family): `.fabro/`, `.mulch/`, `.agents/`, `scripts/`,
  `justfile`. FILE TOOLS (read_file, write_file, edit_file, glob discovery)
  fail on them — reads and writes both; the shell is unaffected (reads AND
  writes succeed through grep, sed, cat, python3 heredocs — the documented
  escape hatch). The `ml` and `just` commands keep working through
  the shell.
- Repo wiring — visible, but never modify without the item saying so
  explicitly: `AGENTS.md`, `CLAUDE.md`, `docs/`, `Cargo.toml`, and the
  workspace manifests.
- Graph source: the workflow directory's `workflow.fabro` (DOT); `workflow.toml` wires runtime settings only — never probe it for edges.
- Merge-target branch — the branch this item loop's run PRs integrate
  into: `origin/denkhaus`. `origin/main` is only the upstream fabro-sh
  mirror and never carries the loop's item merges; branch-sensitive
  checks (e.g. the already-landed greps) must fetch and
  grep the merge-target branch named here, never the mirror.
- Work-item tracker (ADR-0032): the project's work items live in the
  central tracker behind the fabro server; agent access is EXCLUSIVELY
  the engine-mediated run tools — `fabro_work_next` (the atomic claim;
  the candidate source IS the tool grant, never a list), `fabro_work_show`,
  `fabro_work_list` (with `query`, a title+body search including closed
  items), `fabro_work_create`, `fabro_work_update`, `fabro_work_close`,
  `fabro_work_release`. No tracker CLI, no tracker file, no forge client
  (ADR-0019). Item keys are provider-stable and look like
  `<owner>/<repo>#<n>` — copy them verbatim, never truncated. The
  develop line claims lane `fabro` — the lane is the ownership switch
  (ADR-0018's assignee, carried into ADR-0032's lane labels).
- Quality gate: `just qualitygate` — a `qualitygate` recipe in the project
  justfile. The workflow stays agnostic about what the gate checks.
- Rust toolchain pin (fmt/clippy run on the pinned nightly; tests run
  through `cargo nextest`):
  - `cargo +nightly-2026-04-14 fmt -p <touched-crate>`
  - `cargo +nightly-2026-04-14 clippy -p <touched-crate> --all-targets -- -D warnings`
- Stage journal: `.fabro/journal/<run_id>.jsonl` — one JSON record per stage
  completion; the fallback source for recovering a run's claimed item key
  (shell grep for the `#`-key shape when a run goal names none).
- Engine credential surfaces (ADR-0019 review axis): the engine injects
  `GITHUB_TOKEN` into every agent shell call (`resolve_workflow_env` in
  `lib/components/fabro-workflow/src/services.rs`) and runs a git credential
  bridge (`lib/components/fabro-workflow/src/git_bridge.rs`) — agent-reachable
  capability that is invisible from the container environment alone.
