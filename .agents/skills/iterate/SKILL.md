---
name: iterate
description: >-
  Drive one fabro development cycle end to end on the fork branch denkhaus:
  orient on line state (production server, friction score, interrupted
  cycles), select the highest-value seed, delegate implementation to the
  autonomous line (the session claims no seeds), grill pivotal forks,
  review landed diffs (code-review, fork presence pins, fact-checks),
  deepen conditionally, integrate (commit, push-gate, deploy windows),
  and reflect (cost review, seed filing, line-watch heartbeat). Use when
  the user invokes /iterate, asks for a development cycle or the next
  cycle step, or wants cycle work continued after an interruption. Local
  session instrument only; fabro workflows never load it. Complements the
  integrate skill: iterate owns the OUTGOING cycle, integrate the INCOMING
  side.
---

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

- One world: everything works on the LINE branch `denkhaus` in the main
  checkout. VCS layer is the GitButler experiment (2026-10-03): the checkout
  itself sits on `gitbutler/workspace` and the line is the APPLIED virtual
  branch `denkhaus` — `but status` must show it applied (this replaces
  `git branch --show-current`). All write operations go through `but`;
  read-only git inspection (`git log`, `git show`, `git diff`) stays
  allowed. Runs execute on the PRODUCTION server `https://mirtuell.net` —
  every line query carries `--server https://mirtuell.net`. The local
  server (127.0.0.1:32276) is for TESTS only. Apply/unapply of branches
  plays the role of branch switches; git worktrees never.
- `but branch update denkhaus` BEFORE reading tracker state when another
  machine may have run the line (integrates origin/denkhaus into the applied
  branch, pull-rebase) — the tracker view is branch-local. Never apply,
  unapply, or update branches while a background cargo runs (the tree is
  rewritten and a mixed build invalidates the whole run).
- Open-PR sweep BEFORE anything else (user directive 2026-10-02, PR #359
  lesson): `gh pr list --repo denkhaus/fabro --state open` plus
  `gh pr checks <n>` for each. A run PR (`fabro/run/*` branches) with a RED
  dogfood-gate blocks auto-merge, fakes in-flight state, and wedges the
  push gate — diagnose and repair it BEFORE selecting/dispatching a seed:
  snapshot drift -> accept the snapshot ON THE RUN BRANCH and push (auto-
  merge then lands it); deeper breakage -> salvage-or-close decision.
  A merged-on-another-machine PR means the local view is stale: pull again.
  Green-but-waiting PRs: note them, do not touch.
- Line state: `fabro ps` (mirtuell). A running pass plus an empty
  `seeds ready --assignee fabro` queue is fine (fail-closed park); a parked
  pass with a non-empty queue, a parallel pass, or a lost `on_overlap:skip`
  is an incident. While the workflow works the tracker, this session
  claims nothing — one line, one executor.
- Interrupted cycle: reconstruct BEFORE selecting — `git status` plus
  `seeds list --status in_progress` name the mid-flight work; continue it.
- Sprint ledger (session-LOCAL): read `iterate-state.json` in the repo
  ROOT (user directive 2026-10-02 evening: root, never `.fabro/`). The
  sprint system is this SESSION's working mode — the LINE has nothing to
  do with sprints and
  implements none of the mechanics (no ledger writes in lane assets, no
  reflection parks, no counters; fabro-cadd rejected, PRs #359/#360 closed
  not-planned). If `sprints_reflected < sprints_completed`, the pending
  reflection runs FIRST (Phase 6 short form) — no new sprint starts
  unreflected. If `sprints_completed % 3 == 0 && last_arch_review_at_sprint
  != sprints_completed`, the architecture gate is DUE: surface to the user
  and fire the LOCAL improve-codebase-architecture agent (user directive
  2026-10-02: the architect WORKFLOW stays deactivated on the line — never
  the api trigger; the local agent's top recommendations are filed as
  seeds automatically). ALL seeds filed from the architecture pass are
  worked LOCALLY by this session; loop-asset-surface findings may go to
  the loop workflow — none go to the develop line (user directive
  2026-10-02 evening).
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
- Dispatch is one ceremony: ASSIGN, VERIFY, FIRE. A seed named in a run
  goal must be claimable by the fail-closed picker BEFORE the fire
  (`@fabro` for develop work, `@loop` for loop assets) — an unassigned
  seed in a goal makes the planner silently substitute the next claimable
  seed while the implementer still follows the goal text, closing the
  WRONG seed (claim/claim-check mismatch class). Verify with
  `seeds show <id>` (assignee set) in the same breath as the fire; the
  goal text names the seed id AND its one-line topic.
- Executor decision (SUPERSEDED 2026-10-02 evening, local-first model):
  invoking /iterate means the session works LOCALLY — the sprint loop
  (claim, implement, review, reflect) IS this session's working mode (the
  conductor-role split of directive 2026-09-06 is retired). DEFAULT
  ASSIGNMENT (user directive 2026-10-02 late): EVERYTHING is implemented
  by this session locally; ONLY loop-asset work (surfaces
  `.fabro/workflows/**`, `.fabro/scripts/**`, `scripts/**`, justfile,
  fabro-test rigs) is assigned `@loop` for the loop workflow (which works
  it WITHOUT sprint framing). improve-codebase-architecture-filed seeds
  are worked LOCALLY. Develop-line dispatch is gone as a default; it
  stays possible only as an explicit user-routed exception. This session
  still orients, grills pivotal forks (writing the agreed design INTO the
  seed before implementation), reviews, reflects, reports.

## Phase 2 — Build (local-first; delegation only by explicit choice)

- File the seed with the agreed design BEFORE any code exists — design
  decisions live in the tracker, never only in chat or the diff.
- LOCAL builds follow the sprint discipline: claim
  (`seeds update --status in_progress`), implement under the mechanical
  gates of the touched surface (Rust: rust-style-guide + guideline pages
  loaded and NAMED before the first edit cell), verify with real
  commands, close with evidence. The full direct-fix recipe below
  (lint first, dry-runs, unpiped exit codes) is binding for local work.
- Delegated builds (the explicit-choice case): feed implementation seeds
  with pointers — files, trait seams, guideline pages.
- Fork wiring (2026-09-30, 3fce cycle): a fix on a forked dep repo is NOT
  shipped until every consuming workspace's Cargo.lock pins the new rev -
  `branch =` patch pins move only on `cargo update`; bump the lock in the
  SAME change (a fabro lock still at the pre-fix petri rev nearly shipped a
  half fix and probe-15 red). Patch BOTH crates of a multi-crate fork repo
  (pebble-coding-agent AND pebble-agent) - a single entry splits shared
  traits into two copies and fails unification.
- Rust work (when directly assigned): mechanical gate — read SKILL.md AND
  the guideline pages covering the diff in the SAME turn, before the first
  Rust edit cell; name the pages in the cycle report.
- Rust gate-repair pushes (fixing a RED dogfood-gate): verify with the
  FULL workspace program locally BEFORE the push — `cargo +<pin> clippy
  --locked --workspace --all-targets -- -D warnings` plus the touched
  crates' `cargo nextest run --profile ci --no-fail-fast`. The gate aborts
  at the first failing crate, and nextest at the first failing test, so
  later-crate breakage and further snapshot drift stay invisible through
  any number of 16-min CI rounds (fabro-1b2a: three stacked layers, two
  snapshot rounds; fabro-9707 tracks the CI-side fix).
- Direct-fix verification recipe (before any push): lint first
  (`just lint-nu` for nu), then DRY-RUNS — positive AND negative — each
  as its own shell call, exit codes read from the process (never behind
  a pipe: `$?` reads the last pipeline member, not the script). Nu
  interpolated strings treat `word:` before a `(` as a command call —
  reword such phrases; parse-clean does not mean run-clean.
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
- Local full-crate verification runs with `--profile ci` timeouts: a
  default-profile timeout on a Docker/sandbox test under dev-host
  conditions is an environment artifact (known class), not a regression —
  rerun isolated with the ci profile before treating it red.
- After any non-PR integration (emergency squash, manual land): sweep
  for the platform-created run PR and the run branch — a leftover open PR
  blocks the push-gate and fakes in-flight state. Close the PR as
  already-integrated citing the landed sha; delete the branch.
- Direct pushes to the line branch skip dogfood-gate (it runs on PRs only)
  and reds accumulate invisibly until the next PR pays for them
  (2026-09-30: three stale suites surfaced at once). Run the touched
  crates' tests before a direct denkhaus push, or use a PR vehicle for
  test-affecting work. Never switch branches while a background cargo
  runs - the builder reads live sources and a mixed build invalidates the
  whole run.
- Failed-run salvage (user directive 2026-09-30): when an evaluated run
  FAILED or is a green-lie `succeeded` and its implementer/tester stages
  produced noteworthy work (non-trivial diff), SALVAGE it — `fabro dump
  --output <dir> <run>` (run state is durable on mirtuell), apply the
  stage diff, verify, land via PR citing the run (the PR #337/#339
  pattern), or file a salvage-pointer seed citing run id + dump command +
  stranded checkpoints when direct landing does not fit. Never let
  stranded work vanish silently; never let a failed run close or lose a
  seed (closure follows the publish path — failed runs publish nothing).
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

- Architecture-gate procedure (local agent): delegate the scan to a
  sub-agent with an output contract (candidates to a file: files, scope,
  problem, solution, benefits, deletion-test verdict, strength), curate
  with the fork-scope filter, then file EVERY non-Speculative candidate
  as a seed IMMEDIATELY with owner-based assignment — nothing survives
  only in the report. Render the HTML report (interactive skill's
  artifact) on LOCALHOST only — anything beyond this machine is an
  exposure decision that belongs to the user. Mark the ledger
  (`last_arch_review_at_sprint`) and offer the grilling loop for the
  picked candidate.
- Standing path (user directive 2026-10-02, freeze era): the architect
  WORKFLOW is DEACTIVATED on the line — the architecture gate fires the
  LOCAL improve-codebase-architecture agent instead; its top
  recommendations become seeds automatically, assigned per owner (agent
  local / fabro / loop). The autonomous architect workflow returns only
  on an explicit future user order (then: self-gated by friction, 48h
  cooldown, files its own seeds).
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

- Commit code (`but commit`) BEFORE tracker mutations land. `seeds sync`
  is SAFE again since seeds 0.6.0 with `vcs_manager: gitbutler` in
  `.seeds/config.yaml` (set in the GB repos): sync becomes report-only
  (no git write; prints the changed paths + the ready commit message) —
  take those paths as file IDs via `but diff` and commit explicitly.
  In git-mode repos `seeds sync` behaves as before.
- A `fabro create` whose post-processing dies still created the run:
  capture exactly one run id per intended create and `fabro rm --force`
  duplicates immediately - submitted ghosts count as active runs and wedge
  the push gate.
- Push policy: during the cycle, branch updates and read-only integration
  stay allowed; the PUSH direction is gated to one mechanical decision at
  the END: `nu .fabro/scripts/push-gate.nu` (exit 0 = open: no running
  conductor/develop/revisor pass AND no open run PR). Check and push
  share one cell, the gate runs UNPIPED, and the push itself is
  `but push denkhaus` — `&&` behind a pipe reads the pipe member's exit
  code, not the gate's (2026-10-02 23:00 incident: a
  `gate | tail -1 && git push` pushed straight through a REFUSED verdict
  while a loop pass ran), and `cmd; echo RC=$?; if [ $? -eq 0 ]` reads
  ECHO's status, not cmd's — capture `rc=$?` on the line directly after
  the gated command and branch on `$rc` (2026-10-03 near-miss, arch-branch
  push cell). A repaired gate is validated against a known-active
  line state before its first OPEN verdict is trusted. Incident restore
  may push as soon as no pass runs. Evidence and history: `ml prime git`.
- Deploy windows: deploy only while no conductor pass runs. Pause the
  line first (automation replace with FULL body + `If-Match` revision +
  explicit `on_overlap: skip`; re-GET and verify it survived), deploy
  nonblocking, smoke on mirtuell.net, re-enable, monitor via heartbeat.
- Fork releases follow ADR-0025 naming `<frozen-base>-fork.N` — the
  upstream `cargo dev release` path (origin main push) is WRONG for the
  line branch. Until a fork mode exists: manual bump = workspace
  `Cargo.toml` version + `cargo update --workspace` + `just
  image-release` (tag `<version>-<sha>`); N increments once per built
  release; the base moves only with a deliberate intake decision.
- Secret-gated steps (tofu/gopass) are probed with a bounded check
  (`timeout 5 ... gopass show`) BEFORE the step that needs them; the
  store relocks on its own TTL — ask the user to warm it EARLY, not at
  deploy time. Recompute the sandbox-plugin sha per deploy (same pin rev
  usually means the same sha — verify, never assume).
- Production deploy after substantial engine changes (binary-need check:
  any `lib/` path in the merged work): `just image-release` +
  fabro-tofu apply (`cd ~/dev/fabro-tofu`, mise exec tofu). `just up`
  refreshes the LOCAL test stack only. gopass cold cache hangs
  non-interactively — ask the user to warm it before tofu runs.
- After EVERY tofu deploy: commit the fabro-tofu `variables.tf` image pin
  in the same session (2026-09-30 near-miss: the night deploy applied via
  `-var` override, the uncommitted default bump was lost, and the next
  plain `tofu apply` would have rolled production back to a pre-fix
  image). The committed default must name the LIVE image before the
  session ends.
- Bench tooling (fabro-test): script changes land through the one-command
  PR dance (`just land <subject>`), NEVER a manual reset dance; scripts
  with a deterministic-reset step refuse dirty trees. The bench
  (`nu scripts/workbench.nu`, local stack default) is the engine gate —
  a full table with only tracked known-reds (YELLOW, exit 0) is the pass
  state; every fresh red is a finding. Nu scripting pitfalls:
  `ml prime nu` and `ml prime tooling`.

## Phase 6 — Reflect (mandatory, every sprint close)

0. END-OF-SPRINT CEREMONY (user directive 2026-10-02 late): every sprint
   close ends with a Zusammenfassung + Ausblick in the chat, delivered in
   WAIT-WHAT style (the ~/.agents/skills/wait-what re-pitch format):
   give context FIRST so the user instantly re-enters (which sprint just
   closed, which seed, where it landed), then the summary (what was done,
   verification evidence), then the AUSBLICK — what the NEXT sprint will
   work (name the seed and topic) and WHERE the user is needed (decisions,
   gates, credentials, reviews). Simplified short sentences, the repo's
   ubiquitous language (Sprint, Seed, Linie, Ledger, Gate, Loop-Lane);
   chat language stays German. After the summary the next sprint starts
   IMMEDIATELY (backlog order) unless a user gate from the Ausblick is
   blocking it.
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
5. Sprint ledger update (session-LOCAL): counting rule
  (user directive 2026-10-02 evening): ONLY closures this session worked
  LOCALLY with a substantive diff count 1 sprint; docs-only, row-only,
  and verify-only closures count 0. LINE closures (loop lane, delegated
  develop runs) NEVER count — the line is not inside a sprint. Historical
  note: the bootstrap-era count of 3 includes develop-lane closures under
  the superseded lane-blind rule; kept as history, not recounted.
5b. Heartbeat cadence tracks reality: 20m while anything autonomous runs
  (pass, build, deploy chain); 60m when everything is user-gated. Refresh
  the instruction at every phase change — a heartbeat still carrying a
  completed critical path is noise that erodes trust.
6. Line-watch heartbeat (label `line-watch`, interval 10m, follow-up
   delivery): pull, evaluate journals/reviews (premise-checked against
   the tree), dispatch seeds per ADR-0018 with dispatch-dedupe (keep the
   richer seed, close the lesser naming both ids), push through the gate
   (`but push denkhaus`, gate unpiped) with JSONL-dedupe discipline, rootprint correlation for every
   evaluated run, SALVAGE SWEEP per the failed-run salvage rule above for
   every failed/green-lie run, report compactly in German. RLM heartbeats are
   session-scoped: recreate from this spec when missing; if the session
   host rejects heartbeats entirely, ask the user to set the visible
   /heartbeat with this ceremony — never run silently unwatched.

## Standing rules

- NO LANGUAGE SPRAWL (user top priority, directive 2026-10-03): every script invoked by workflows, justfile, or gate/tooling entry points MUST be a nu script. Never add, extend, or split non-nu script assets (the code-review lane's python engine is a standing violation to MIGRATE to nu, not a pattern); architecture scans and sprint plans check the language axis BEFORE proposing splits/refactors.
- Style-guide pair invariant (fabro-6538, 591a decision 2026-10-03):
  `.fabro/skills/rust-style-guide/` is canonical, `.agents/skills/rust-style-guide/`
  its synced mirror. After ANY edit to the canonical side, the session runs
  `just sync-style-guide` (the qualitygate parity battery REDs on divergence).
  Loop runs NEVER write `.agents/**` (fabro-591a decision: no run-scope
  widening — battery-RED routes the sync to the session).
- VCS layer is the GitButler experiment (user directive 2026-10-03): every
  write operation (commit, push, branch, history edit) goes through `but`;
  never run `git add/commit/push/checkout/merge/rebase/stash/cherry-pick`.
  Target is `origin/main` (the frozen ADR-0024 base) — do NOT run
  `but pull` routinely (it would intake upstream); line updates arrive via
  `but branch update denkhaus`, line pushes via `but push denkhaus`.
  `but` does NOT maintain the classic git index — a stale index makes
  `git diff`/`git status` show phantom `MM` entries while the worktree
  matches HEAD (2026-10-03: five loop files looked uncommitted). Proof of
  real change is `git diff HEAD -- <files>`; `git reset` (mixed, index
  only) restores honest views and leaves `but status` untouched.
  Multi-agent note: the experiment exists to run several agents as separate
  virtual branches/stacks in one workspace; report friction in the cycle
  report so the go/no-go stays factual. If the experiment fails, revert the
  but-specific procedures of this skill to git wording.
- Wait budgets derive from observed service latencies, not guesses: a
  bounded wait must outlive the slowest LEGITIMATE stage of what it waits
  for (PR creation after terminal status; required-check duration on a
  PR). A fallback that bypasses a verification path must re-check the
  gated path one last time before engaging, and clean up the ghost it
  creates (open PR, remote branch) so gates downstream inherit nothing.
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
- Seeds whose fix surface is an implementer-hidden loop asset (`scripts/**`,
  `.fabro/**`, `justfile`) are unlandable by the develop line - the fs
  envelope correctly kills the write every pass (fabro-c4be, two envelope
  kills 2026-09-30). Classify them direct work at assignment or ask for a
  deliberate envelope exception.
- DIRECT-FIX PROPOSAL DUTY (user directive 2026-10-01, e62d cycle): at
  filing/triage time, EVERY seed whose fix surface makes it unlandable by
  the line (loop-asset class above) or trivially mechanical (one-file,
  evidence already in hand) is PROPOSED to the user as a direct fix in the
  same turn it is filed - never silently queued. Waiting for a lane slot
  on work this session could land in minutes loses a day per seed
  ("sonst verlieren wir Zeit"). The proposal names the seed, the fix
  surface, why the line cannot or should not take it, and the estimated
  effort; the user decides GO/no.
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
- Upstream posture (ADR-0024, 2026-10-01): the platform base is FROZEN at
  0.362.0-nightly (merge-base 1b4fb1528). No routine merges of fabro main.
  Intake: (a) petri/pebble fork merges only when they compile and pass gates
  against OUR base (skip+record what needs fabro-side changes we lack);
  (b) fabro-main fixes land as bounded, justified cherry-picks (registered
  candidate: the sandbox-side publication rewrite #913). Drift watching is
  informational, never action-triggering. Selectively OFFER general,
  non-strategic fixes upstream (fabro-d485); strategic assets stay
  fork-private. Endgame: convergence or a deliberate wholesale switch —
  keep the .fabro layer portable (seams/presence pins stay binding).
- Fork-feature presence pinning: every durable fork feature lives in a
  fork-only source file wired through minimal one-line seams, with a
  presence test in a fork-only test file plus a touchpoints row —
  upstream merges cannot silently drop it.
- Session/line ownership boundary (user directive 2026-10-02 evening,
  fabro-cadd lesson): session bookkeeping — the sprint ledger, the
  reflection invariant, the arch-gate cadence — is iterate-skill-LOCAL
  and lives at the repo root (`iterate-state.json`). Any seed proposing
  to port session bookkeeping or control-loop ceremony into line assets
  (`.fabro/workflows/**`, `.fabro/scripts/**`, `scripts/**`) is
  needs-user/grill-first — NEVER assigned to a lane unseen. The line's
  fail-closed invariants are its own (tracker, envelope, gates), never
  the session's homework. The architect agent is local, permanently.
- Boundaries: mulch = expertise, seeds = actionable work, ADRs =
  decisions, this skill = process. Nothing stays in chat that belongs in
  one of them. Fabro-specific Rust rules (test-support feature, strum,
  import style, shell_quote) sit ON TOP of rust-style-guide.
