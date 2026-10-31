## PROJECT_FACTS — the repo-specific values this workflow runs on

This block is the ONE place the loop workflow carries facts about THIS
repository (ADR-0013; the prompts that include this block stay
project-agnostic). Porting the workflow means editing this file, not the
prompts. A stale value here is loop friction: report it in the journal,
never silently work around it.

- Primary work surface — where loop-item work lands: the loop's own
  machinery (ADR-0008 one-unit): `.fabro/workflows/**` (graphs, prompts,
  schemas, scripts), `.fabro/scripts/**`, root `scripts/**`, `justfile`
  (the legacy `.seeds/` tracker left the tree with the cutover — its
  frozen history lives in the tracker repo under
  `denkhaus/work/archive/denkhaus-fabro/`; nothing reads it since
  denkhaus/work#990) —
  plus ONE lib/ exception: `lib/components/fabro-dot/src/snapshots/**`,
  the snapshot file mechanically derived from the workflow graphs (accept
  it with `cargo insta accept` when the dot-snapshot gate tier goes RED
  on your graph-shape change; ship the .snap update in the same diff).
  The loop implementer's fs envelope pins EXACTLY this set; the loop
  tester's run-scope check refuses every other path.
- Out-of-lane surfaces — never touched by a loop run: product code
  (`lib/`, `apps/`, `docs/`, `lib/packages/`) EXCEPT the fabro-dot
  snapshot dir above, session-level skills
  (`.agents/`), expertise (`.mulch/`). An item whose primary surface is
  one of these is misrouted — see the planner prompt.
- Merge-target branch — the branch this item loop's run PRs integrate
  into: `origin/denkhaus`. `origin/main` is only the upstream mirror;
  branch-sensitive checks (the already-landed greps) must fetch and grep
  the merge-target branch named here, never the mirror.
- Work-item tracker (ADR-0032): the project's work items live in the
  central tracker behind the fabro server; agent access is EXCLUSIVELY
  the engine-mediated run tools — `fabro_work_next` (the atomic claim;
  the candidate source IS the tool grant, never a list), `fabro_work_show`,
  `fabro_work_list` (with `query`, a title+body search including closed
  items), `fabro_work_create`, `fabro_work_update`, `fabro_work_close`,
  `fabro_work_release`. No tracker CLI, no tracker file, no forge client
  (ADR-0019). Item keys are provider-stable and look like
  `<owner>/<repo>#<n>` — copy them verbatim, never truncated; legacy
  items imported from the pre-ADR-0032 tracker keep resolving by their
  legacy id through `fabro_work_show`. The loop line claims lane `loop`
  — the lane is the ownership switch (ADR-0018's assignee, carried into
  ADR-0032's lane labels).

- Deterministic gate (loop lane): `nu .fabro/workflows/loop/scripts/loop-gate.nu` — the tester step owns it. The battery: validate every
  workflow graph (petri admission), lint every nu script, prompt-lint
  literal hygiene, run-scope (diff touches only loop assets),
  seed-spec-paths (every loop-asset path the claimed seed's spec NAMES
  must appear in the run diff — an empty-edit or wrong-file pass REDs),
  and — only
  when the diff changes a workflow graph — the fabro-dot
  checked-in-workflows snapshot test (accept drift with
  `cargo insta accept` before publish). NEVER the product compile tier
  otherwise; rust fmt only when .rs files appear.
- Stage journal: `.fabro/journal/<run_id>.jsonl` — one JSON record per
  stage completion; the fallback source for recovering a run's claimed
  item key.
- Engine credential surfaces (ADR-0019 review axis): the engine injects
  `GITHUB_TOKEN` into every agent shell call (`resolve_workflow_env` in
  `lib/components/fabro-workflow/src/services.rs`) and runs a git
  credential bridge (`lib/components/fabro-workflow/src/git_bridge.rs`)
  — agent-reachable capability invisible from the container env alone.
