## PROJECT_FACTS — the repo-specific values this workflow runs on

This block is the ONE place the loop workflow carries facts about THIS
repository (ADR-0013; the prompts that include this block stay
project-agnostic). Porting the workflow means editing this file, not the
prompts. A stale value here is loop friction: report it in the journal,
never silently work around it.

- Primary work surface — where loop-seed work lands: the loop's own
  machinery (ADR-0008 one-unit): `.fabro/workflows/**` (graphs, prompts,
  schemas, scripts), `.fabro/scripts/**`, root `scripts/**`, `justfile`,
  and the tracker file `.seeds/issues.jsonl`. The loop implementer's
  fs envelope pins EXACTLY this set; the loop tester's run-scope check
  refuses every other path.
- Out-of-lane surfaces — never touched by a loop run: product code
  (`lib/`, `apps/`, `docs/`, `lib/packages/`), session-level skills
  (`.agents/`), expertise (`.mulch/`). A seed whose primary surface is
  one of these is misrouted — see the planner prompt.
- Merge-target branch — the branch this seed loop's run PRs integrate
  into: `origin/denkhaus`. `origin/main` is only the upstream mirror.
- Issue tracker: the `seeds` CLI (Seeds, git-native in `.seeds/`). The
  loop line works EXCLUSIVELY on seeds assigned to assignee `loop` —
  the assignee is the ownership switch (ADR-0018, symmetric to @fabro).
  Seed ids carry the prefix `fabro-` (e.g. `fabro-37a6`). The supported
  read path is `seeds show <id> --format json`; never parse the raw
  tracker file by hand. Exact command reference (never invent flags):

| Command | Purpose |
|---|---|
| `seeds ready --assignee loop --limit 200` | Unblocked open seeds ASSIGNED TO loop — the ONLY candidate source. If it answers the question, do NOT also run `seeds list`. ALWAYS pass `--limit 200`: the default 50 silently truncates. |
| `seeds list --format json --assignee loop --limit 200` | Full tracker picture, loop-assigned only (only when `seeds ready` was not enough). NEVER list without the assignee filter. |
| `seeds show <id> --format json` | One seed in full (the supported path). |
| `seeds update <id> --status in_progress --assignee loop` | Claim (the exact claim form). Takes NO `--format` flag. |
| `seeds update <id> --description "<full corrected body>"` | Record a stale-spec correction BEFORE the claim — `--description` replaces the body wholesale: re-emit the FULL corrected body including the existing `Basis:` line. Takes NO `--format` flag. |
| `seeds close <id>` | NEVER yours — the deterministic Closeout step closes approved seeds — with exactly ONE exception: the planner's superseded-close `seeds close <id> --reason "superseded: fix landed in <sha>"` when a fix commit referencing the seed is already in base history and the acceptance criteria hold. Every other close form is forbidden to every role. |

- Deterministic gate (loop lane): `nu .fabro/workflows/loop/scripts/loop-gate.nu` — the tester step owns it. The battery: validate every
  workflow graph (petri admission), lint every nu script, prompt-lint
  literal hygiene, run-scope (diff touches only loop assets), rust fmt
  only when .rs files appear. NEVER the product compile tier.
- Sprint ledger (iterate model, ADR-0024): `.fabro/iterate-state.json`
  counts sprints — one closed seed with a substantive diff = 1 sprint,
  ANY lane; verify-only closures count 0. The deterministic Closeout
  step updates it on every seed close (counter + line-side short
  reflection ride the close; `nu .fabro/scripts/iterate-ledger.nu` is
  the only reader/writer). The loop tracker guard parks the run
  ("Sprint unreflected", deadlock exit — seeds stay open) while
  `sprints_reflected < sprints_completed`: run the session-side short
  reflection first. When closeout prints `gate_due: true` (every 3rd
  sprint), the LOCAL improve-codebase-architecture agent pass is due —
  record it with `iterate-ledger.nu --mode arch-reviewed`.
- Stage journal: `.fabro/journal/<run_id>.jsonl` — one JSON record per
  stage completion; the fallback source for recovering a run's claimed
  seed id.
- Engine credential surfaces (ADR-0019 review axis): the engine injects
  `GITHUB_TOKEN` into every agent shell call (`resolve_workflow_env` in
  `lib/components/fabro-workflow/src/services.rs`) and runs a git
  credential bridge (`lib/components/fabro-workflow/src/git_bridge.rs`)
  — agent-reachable capability invisible from the container env alone.
