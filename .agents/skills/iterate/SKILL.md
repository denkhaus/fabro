# /iterate (fabro only)

One development cycle, end to end. The user starts a cycle by invoking
/iterate; the agent drives everything else. Chat replies in German,
all written artifacts in English.

Scope: LOCAL session instrument for the human-side agent. Fabro workflows
never load or reference this skill; skills a run's agent stages need are
vendored into `.fabro/skills/<name>/`.

## Routing — where knowledge lives

This file carries the LOOP and the BINDING RULES. Expertise lives in the
stores it belongs to; reach them by phase need:

- `ml prime <domain>` — incident expertise and conventions. Domains you
  will need: `rust` (edit mechanics), `testing` (verification discipline),
  `git` (push/PR discipline), `tooling` (tracker hygiene, bench landing),
  `nu` (scripting pitfalls), `engine` (over limit — prefer specific domains).
- `seeds show <id>` / `seeds search <term>` — actionable work, one keyword
  per search (AND-strict).
- ADRs in `docs/lab/adr/` — decisions. Ones this skill leans on: 0008
  (graph+prompts+scripts as one unit), 0009 (stage-envelope family order),
  0012 (dogfooding), 0015 (serialization, one line one executor), 0017
  (tool-agnostic engine), 0018 (ownership boundary), 0019 (capability
  gate), 0021 (fork_line_recovery pattern), 0022 (judgment pre-screens).
- `docs/internal/*-strategy.md` + AGENTS.md — read before touching the
  matching area (logging, events, testing, secrets, migrations, errors,
  React effects).
- `.agents/skills/rust-style-guide` — THE binding Rust policy, identical
  for upstream and fork code.

## Phase 0 — Orient (always, cheap)

- One world: everything works on branch `denkhaus` in the main checkout
  (`git branch --show-current` must say `denkhaus`). Runs execute on the
  PRODUCTION server `https://mirtuell.net` — every line query carries
  `--server https://mirtuell.net`. The local server (127.0.0.1:32276) is
  for TESTS only. Branch switches happen in the main checkout; git
  worktrees never.
- `git fetch` + `git pull --ff-only` BEFORE reading tracker state when
  another machine may have run the line — the tracker view is branch-local.
- Line state: `fabro ps` (mirtuell). A running pass plus an empty
  `seeds ready --assignee fabro` queue is fine (fail-closed park); a parked
  pass with a non-empty queue, a parallel pass, or a lost `on_overlap:skip`
  is an incident. While the workflow works the tracker, this session
  claims nothing — one line, one executor.
- Interrupted cycle: reconstruct BEFORE selecting — `git status` plus
  `seeds list --status in_progress` name the mid-flight work; continue it.
- Tracker reads always carry `--limit 500` (`seeds list` caps at 50
  silently). `seeds ready` for candidates; `seeds show <id>` for ids.
- Rootprint (skill `rootprint`) observes the production server: filter on
  body text, `severity_text` is unreliable; INFO is not ingested, so run
  timelines come from `fabro ps`/events. For EVERY run under evaluation,
  correlate the run's rootprint window with its journal painpoints and
  land findings in ONE seed carrying both halves.
- Friction score: run `nu .fabro/scripts/friction-score.nu` once. Verdicts:
  `normal` (<0.30), `grind` (0.30–0.60), `architecture-due` (>=0.60) —
  gates Phase 4; journal the components when a cycle ends in grind.

## Phase 1 — Select

- Pick the highest-value open seed respecting family order (ADR-0009:
  900e -> 47b5 -> e47c -> e804 -> ba96), blockers (`!`), and the user's
  focus.
- Pivotal or unclear choice/design: grill-with-docs FIRST (grilling +
  domain-modeling; evidence = CONTEXT.md, ADRs, seeds, code, guide).
  Weichenstellende decisions belong to the user. Public-contract NAMES
  (graph attributes, API fields, tool names, CLI flags) are pivotal too:
  name the capability, surface naming for user decision before the first
  commit.
- Plan against the guide: name the guideline pages the design must
  satisfy. A design that must deviate is itself a pivotal fork.
- Executor decision (directive 2026-09-06): this agent implements NO
  seeds — product and engine/platform work goes to the autonomous line.
  This session orients, monitors, grills pivotal forks (writing the agreed
  design INTO the seed before the workflow implements it), reviews landed
  diffs, revises the autonomous workflows, reflects, reports. The
  conductor claims seeds; an agent-side claim would empty the planner's
  queue. Claim (`seeds update --status in_progress`) only in direct-mode
  bootstrapping exceptions (see Standing rules).

## Phase 2 — Build (delegation mode: feed the seed, not the diff)

- File the seed with the agreed design BEFORE any code exists — design
  decisions live in the tracker, never only in chat or the diff.
- Feed implementation seeds with pointers: files, trait seams, guideline
  pages. Platform/engine changes are line work too (PR #28 proved it).
- Rust work (when directly assigned): mechanical gate — read SKILL.md AND
  the guideline pages covering the diff in the SAME turn, before the first
  Rust edit cell; name the pages in the cycle report.
- Loop assets are deterministic-script-first: a prompt clause requiring
  judgment over mechanical data becomes a script that prints a verdict
  (`.fabro/scripts/`, `just lint-nu` for new nu scripts; prompts keep only
  the call + verdict routing). Never write and execute an edited script in
  one shell call.
- Probe rigs: workspace-relative hook/stage scripts resolve against the
  RUN TARGET checkout — land the probe on the target branch first or
  create with `--target OWNER/REPO@<probe-branch>`. Nu verify scripts get
  positive AND negative dry-runs before any run.
- `just up` ships the working tree into the local image: run it from a
  landed/clean state. A handoff's stack claim is verified
  (`.../install/session`: 401 = unconfigured, 404 = configured).
- Edit mechanics (anchors above items, one-cell assert->write, slice
  uniqueness, compiler-as-worklist, fixture verification): `ml prime rust`.

## Phase 3 — Review

- Run the code-review skill on the diff since the base point (standards
  axis = rust-style-guide + strategy docs; spec axis). Freeze the diff to
  a patch file first; both axes review the frozen snapshot.
- Proof of work: the cycle report NAMES the guideline pages loaded for
  the diff and the verification commands actually run.
- Fork-feature regression check: run the fork-only presence suites
  (`cargo nextest run -p fabro-workflow -- fork_seam` + newer fork-only
  files) on the reviewed tree. Every NEW fork feature carries a presence
  pin (fork-only test file + touchpoints row); a pin-less fork feature
  ships only with a filed seed. Red fork-only test = landed fork feature
  regressed: fix or revert, never relax the test.
- Landed prompt diffs get a FACT-CHECK against repo reality: claimed
  branches, paths, command behavior. Prompt hygiene: workflow prompts
  land WITHOUT seed-id literals, run ids, PR numbers, commit shas, dated
  cost narratives, or machine-specific paths (`.fabro/workflows/**`);
  branch/merge facts belong in PROJECT_FACTS. The mechanical net is the
  prompt-lint evidence ban.
- Verify a reviewer's factual premise in code before fixing; the same for
  revisor seed citations (check the cited seed's premise AND
  implementation status). A closed seed whose demand is invisible in any
  diff: grep `.fabro/journal` + `.fabro/revisions` for the id before
  reporting it lost; an undocumented closure is itself a finding.
- Tool-call misuse by agents is BOUNDARY evidence, not prompt material:
  fix the boundary (validation naming property paths, contract
  ergonomics), file the boundary seed in the same session.
- Revise the autonomous workflows after every delegated cycle: inspect
  the runs' journals and stage outcomes; every painpoint lands as a seed,
  a skill edit, or an explicit no-action note in the cycle report.
  `fabro ask <run-id>` improve-review occasionally (~every 5th cycle).
- Verification discipline (unpiped commands, full reruns, sequential
  load, failure-set diffs, clean-tree controls, simulation safety,
  e2e-profile traps): `ml prime testing`.

## Phase 4 — Deepen (conditional)

- Standing path: the autonomous architect workflow fires on its own
  schedule (self-gated by friction, 48h cooldown) and files its own
  seeds. Fire it or wait for cron BEFORE any manual
  improve-codebase-architecture pass; the manual skill is the fallback
  (server down, workflow broken) or on explicit user request.
- Architect scope is binding: findings restructure ONLY the fork's own
  surface (`.fabro/workflows/**`, `.fabro/scripts/**`, fork-only files,
  our tooling). Upstream-owned `lib/**`/`apps/**` is observable, never
  restructurable. Apply the same filter when reviewing architect findings.
- Deepen when review surfaced structural smells OR the Phase 0 verdict is
  `architecture-due`. `grind` deepens only on smells; `normal` never
  deepens on score alone. Judge proposals against the guide and
  codebase-design vocabulary. The AUTONOMOUS architect writes markdown
  reviews + seeds only — the interactive skill's HTML report is
  exclusively the interactive skill's artifact.

## Phase 5 — Integrate

- Commit code BEFORE `seeds sync` (sync sweeps staged changes + `.seeds`
  only). Never `seeds sync` inside a workflow stage. Line-watch closes
  through `seeds close --reason` with the reason appended to the body
  first.
- Push policy: during the cycle, pulls and read-only integration stay
  allowed; the PUSH direction is gated to one mechanical decision at the
  END: `nu .fabro/scripts/push-gate.nu` (exit 0 = open: no running
  conductor/develop/revisor pass AND no open run PR). Check and push
  share one cell. A repaired gate is validated against a known-active
  line state before its first OPEN verdict is trusted. Incident restore
  may push as soon as no pass runs. Evidence and history: `ml prime git`.
- Deploy windows: deploy only while no conductor pass runs. Pause the
  line first (automation replace with FULL body + `If-Match` revision +
  explicit `on_overlap: skip`; re-GET and verify it survived), deploy
  nonblocking, smoke on mirtuell.net, re-enable, monitor via heartbeat.
- Production deploy after substantial engine changes (binary-need check:
  any `lib/` path in the merged work): `just image-release` +
  fabro-tofu apply (`cd ~/dev/fabro-tofu`, mise exec tofu). `just up`
  refreshes the LOCAL test stack only. gopass cold cache hangs
  non-interactively — ask the user to warm it before tofu runs.
- Bench tooling (fabro-test): script changes land through the one-command
  PR dance (`just land <subject>`), NEVER a manual reset dance; scripts
  with a deterministic-reset step refuse dirty trees. The bench
  (`nu scripts/workbench.nu`, local stack default) is the engine gate —
  a full table with only tracked known-reds (YELLOW, exit 0) is the pass
  state; every fresh red is a finding. Nu scripting pitfalls:
  `ml prime nu` and `ml prime tooling`.

## Phase 6 — Reflect (mandatory, every cycle)

1. Cost review: what took longer than it should, what needed retries,
   what was missing at decision time. EVERY finding that implies a
   behavior change becomes an edit to THIS skill; domain knowledge goes
   to mulch (`ml record ...`, then `ml sync` — real insights only);
   durable preferences to memory. No finding stays unrecorded.
2. New demands become seeds (`seeds create`), searched-first for
   duplicates. Ownership per ADR-0018: unassigned by default; `@fabro`
   only as a proposal for clearly-line work; design forks and
   user-decisions get `needs-user`. Shell-safe bodies and tracker
   roundtrips: `ml prime tooling`.
3. Note open forks ahead for the next grill-with-docs.
4. Cycle report: outcome, verification evidence (commands run), seeds
   filed/closed — each with a ONE-LINE description, never a bare id —
   and ALWAYS an ASSIGNMENT PENDING section: every unassigned seed with
   a one-line @fabro recommendation. Categorize every revisor seed.
5. Line-watch heartbeat (label `line-watch`, interval 10m, follow-up
   delivery): pull, evaluate journals/reviews (premise-checked against
   the tree), dispatch seeds per ADR-0018 with dispatch-dedupe (keep the
   richer seed, close the lesser naming both ids), push through the gate
   with JSONL-dedupe discipline, rootprint correlation for every
   evaluated run, report compactly in German. RLM heartbeats are
   session-scoped: recreate from this spec when missing; if the session
   host rejects heartbeats entirely, ask the user to set the visible
   /heartbeat with this ceremony — never run silently unwatched.

## Standing rules

- Judgment pre-screens (ADR-0022): advisory, fail-open, via
  `.fabro/scripts/judgment.nu`; thresholds only after the fabro-d4c6
  evaluation report exists; logs to
  `~/.local/state/fabro-judgments/<date>.jsonl`.
- Security-hole closures are DIRECT agent work, never line work: the
  fixer must not stand in the trust circle being closed. Full mechanical
  gate applies.
- Capability gate (ADR-0019): agent sandboxes get least privilege; all
  GitHub writes are engine-mediated; any seed/PR changing a tool,
  credential, or permission in an agent-reachable surface is needs-user
  until the user approves; reviewers and line-watch block such changes on
  sight. A merged capability change without a recorded user decision is
  reverted, not ratified.
- Ownership (ADR-0018): the line works ONLY `@fabro`-assigned seeds
  (fail-closed picker). The revisor files seeds UNASSIGNED. Emergencies
  bypass the picker: line down -> agent repairs directly with the user's
  knowledge, seed filed retroactively.
- Line-health watchpoints: verify `on_overlap=skip` survived any
  automation change; a parallel pass means the policy was lost; a parked
  pass with a non-empty backlog is surfaced, never bulk-assigned behind
  the user's back. Landed-work check: after each pass, confirm the child
  PR merged into `denkhaus` (base branch = denkhaus) and the work commits
  appear in origin/denkhaus.
- Tool-agnostic engine (ADR-0017): engine components never reference
  project-scope tooling by name; bootstrap lives in project artifacts.
- Upstream posture: we offer nothing until upstream reacts to our open
  issues/PRs. Upstream drift threshold 5 minor versions before action.
- Fork-feature presence pinning: every durable fork feature lives in a
  fork-only source file wired through minimal one-line seams, with a
  presence test in a fork-only test file plus a touchpoints row —
  upstream merges cannot silently drop it.
- Boundaries: mulch = expertise, seeds = actionable work, ADRs =
  decisions, this skill = process. Nothing stays in chat that belongs in
  one of them. Fabro-specific Rust rules (test-support feature, strum,
  import style, shell_quote) sit ON TOP of rust-style-guide.
