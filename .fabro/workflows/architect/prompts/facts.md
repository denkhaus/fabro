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
  `reviews/<YYYY-MM-DD>.md` (the analyze stage's skill-grounded review) and
  `<YYYY-MM-DD>-pass.md` (cooldown marker written by the file stage; its
  presence within 48h parks the survey).
- Stage journal — `.fabro/journal/<run_id>.jsonl`: one JSON record per
  stage completion. `nu .fabro/scripts/loop-digest.nu --days 7`
  aggregates EVERY journal into the loop-wide digest (workflow derived
  from the node signature: planner=develop, select=revisor,
  analyze=architect) — the architect's primary system evidence.
- Decision records — ADRs live in `docs/lab/adr/`; strategy docs
  (logging, events, testing, secrets, migrations, error handling) live in
  `docs/internal/`. Read them via shell (they are fs_hide-bound for FILE
  tools in this flow).
- Merge-target branch — `origin/denkhaus`: the branch this line's run
  PRs integrate into.
- CHANGE SURFACE (user directive 2026-09-17, binding): this fork lives on
  upstream (`upstream` -> fabro-sh/fabro) and CANNOT restructure the
  upstream-owned codebase — a large refactor of `lib/**`/`apps/**` upstream
  files would tear apart at the next upstream merge. Architectural changes
  target ONLY: (a) `.fabro/workflows/**` and `.fabro/scripts/**` (our loop
  assets, fully ours), (b) fork-only files (the established pattern:
  `fork_seam_tests.rs`, `fork_line_recovery.rs` — listed in
  `.agents/skills/merge-upstream/references/touchpoints.md`) wired through
  minimal one-line seams, (c) our own tooling/docs. A finding that proposes
  restructuring upstream-owned code is OUT OF SCOPE: reframe it to the
  fork surface (e.g. move fork-added tests OUT of upstream monolith files
  into fork-only modules) or drop it with the constraint named in the
  journal.
- UPSTREAM-FORK INTERPLAY LENS (user directive 2026-09-18, binding —
  extends CHANGE SURFACE): the fork/upstream relationship is a STANDING
  analysis axis in EVERY consideration and proposal, not a final filter
  applied at distillation. Every review walk classifies the files it
  touches as upstream-owned (`git ls-tree upstream/main -- <path>` —
  empty output means fork-owned) vs fork-owned; every surviving finding
  names its upstream interplay: the target surface (fork-owned file /
  loop asset / minimal seam / content-only edit of an upstream-owned
  file) and, whenever an upstream-owned file is touched at all, that
  file's upstream churn rate. Negative example (fabro-90ae, rejected
  2026-09-18): a "locality" proposal that split upstream's hottest file
  (`server/tests.rs`, 112 upstream commits/12w) into 19 fork modules —
  pure recurring merge tax; only a pre-merge close of the offending PR stopped
  it. The constructive direction is fabro-ab8e: extract fork additions
  into fork-owned files, upstream files stay structurally
  upstream-identical.
- Engine-provided credentials (ADR-0019 review axis): the engine injects
  `GITHUB_TOKEN` into every agent shell call (`resolve_workflow_env` in
  `lib/components/fabro-workflow/src/services.rs`) and runs a git
  credential bridge (`lib/components/fabro-workflow/src/git_bridge.rs`) —
  agent-reachable capability that is invisible from the container
  environment alone. Architecture findings that would add, change, or
  remove such capability file as `needs-user` seeds.
