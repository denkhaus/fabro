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
all written artifacts in English — ADR-0033, hard rule, every repo of
the line (fabro, fabro-test, fabro-tofu): docs, ADRs, catalogs, code
comments, workflow prompts, seed notes, commit messages, PR bodies. A
German file does NOT license German additions; translate the whole file
in the same change. The only exemption is test data that deliberately
exercises non-English model output, named as such.

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
- ADRs in `docs/adr/` — decisions. Ones this skill leans on: 0008
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
  checkout. VCS layer is PLAIN GIT (user decision 2026-10-09: the GitButler
  experiment ENDED — assessment ~14-19h friction over 6 days, zero current
  multi-agent use, permanent sha tax; teardown ran on v1000 the same
  evening: `but teardown --checkout-to denkhaus`, `.git/gitbutler` removed,
  `vcs_manager` key dropped from .seeds/config.yaml, the repo http:but pin
  dropped from .mise.toml). Machine 1's teardown ran
  2026-10-16 (same ceremony); any checkout still showing
  `gitbutler/workspace` under `git rev-parse --abbrev-ref HEAD` is a
  machine that has not torn down yet — run the ceremony FIRST, then read
  state. Runs execute on the PRODUCTION server
  `https://mirtuell.net` — every line query carries
  `--server https://mirtuell.net`. The local server (127.0.0.1:32276) is
  for TESTS only. git worktrees never.
- `git pull --ff-only origin denkhaus` BEFORE reading tracker state when
  another machine may have run the line — the tracker view is
  branch-local. Never checkout/reset the tree while a background cargo
  runs (the builder reads live sources and a mixed build invalidates the
  whole run).
- Agent roster FIRST (2026-10-04, second-session incident): call
  `agent_observe.list_agents()` before anything else. Another top-level
  session in THIS repo's cwd means a second /iterate is alive in the same
  checkout: the uncommitted pool, the tracker file and the ledger are then
  SHARED, and one line has one executor per surface. Do not wait: pick a
  disjoint surface (code side vs loop assets), claim in the tracker (a
  sibling's message queue can be full — `agent_message` then refuses), and
  never commit the sibling's files. Foreign uncommitted files in `git
  status` are the sibling's work, not an interrupted cycle of this session. NEVER `git stash` in
  the shared checkout (2026-10-17 incident): the stash stack is GLOBAL
  across concurrent sessions — a `-u` stash grabs every session's WIP and
  a pop restores someone else's, swapping the dirty-set in one second and
  misattributing ownership. Tree-state probes run via read-only checks or
  a temporary branch; suspending or restoring foreign WIP is NEVER this
  session's operation.
- Open-PR sweep BEFORE anything else (user directive 2026-10-02, PR #359
  lesson): `gh pr list --repo denkhaus/fabro --state open` plus
  `gh pr checks <n>` for each. EVERY gh call carries
  `--repo denkhaus/fabro` explicitly — gh's DEFAULT repo in this
  checkout resolves to fabro-sh/fabro (verified 2026-10-17: `gh repo
  view` shows it), a different repo with colliding PR numbers; a bare
  `gh pr merge 427` there hit an unrelated 2026-05 merge (no damage,
  but the checks/greens I read first were the WRONG PR's). A run PR (`fabro/run/*` branches) with a RED
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
  seeds automatically). Two more review gates in the same cadence shape
  (user directive 2026-10-26): if `sprints_completed % 2 == 0 &&
  last_code_review_at_sprint != sprints_completed`, run the CODE-REVIEW
  skill (two axes, parallel sub-agents, frozen diff, tracker specs) over
  the commits since the last code review — fixpoint = the tip AT the end
  of the sprint named in `last_code_review_at_sprint`, derive it with
  `git log`, never HEAD~N — and set
  `last_code_review_at_sprint := sprints_completed`; if
  `sprints_completed % 5 == 0 && last_rust_security_review_at_sprint
  != sprints_completed`, run the RUST-SECURITY skill over the workspace
  diff since its last run and set its marker the same way. `null` means
  never run. All three markers live in iterate-state.json beside the
  notes. ALL seeds filed from the architecture pass are
  worked LOCALLY by this session; loop-asset-surface findings may go to
  the loop workflow — none go to the develop line (user directive
  2026-10-02 evening).
- Tracker reads always carry `--limit 500` (`seeds list` caps at 50
  silently). `seeds ready` for candidates; `seeds show <id>` for ids.
- Rootprint (BASIC KNOWLEDGE, 2026-10-22): production logs are queried
  through the `rootprint` CLI, per the skill at
  `~/.prime/agent/skills/rootprint/SKILL.md` (command map: search, hist,
  fields, values, trace, health; `search -q 'severity_text:ERROR'` is a
  valid field query — the `level` field does not exist). Do NOT read
  `docker logs rootprint-vector`: that container's stdout carries only
  vector's own boot lines; events go to the opentelemetry sink.
  `severity_text` values are still unreliable for catch-alls — filter on
  body text for those. INFO is not ingested, so run timelines come from
  `fabro ps`/events. For EVERY run under evaluation, correlate the run's
  rootprint window with its journal painpoints and land findings in ONE
  seed carrying both halves.
- Friction score: run `nu .fabro/scripts/friction-score.nu` once. Verdicts:
  `normal` (<0.30), `grind` (0.30–0.60), `architecture-due` (>=0.60) —
  gates Phase 4; journal the components when a cycle ends in grind.

## Phase 1 — Select

- Pick the highest-value open seed respecting family order (ADR-0009:
  900e -> 47b5 -> e47c -> e804 -> ba96), blockers (`!`), and the user's
  focus.
- Sprint composition (user directive 2026-10-29): a sprint carries TWO
  to THREE work items, selected together in this phase — the count
  scales with the items' scope and the sprint's diff (one large
  multi-file item may fill the sprint; three small ones may share it).
  A one-item sprint makes the per-sprint code review overkill: the
  review cost is per sprint, so batches amortize it. Grill each item's
  design on its own; work the batch sequentially, closing each item
  with its own evidence.
- EVERY decision point is grilled BEFORE implementation (user directive
  2026-10-12, Grundsatz-review; procedure directive 2026-10-18: run the
  ACTUAL grill-with-docs skill, never an ad-hoc single-shot presentation):
  grill-with-docs FIRST (grilling + domain-modeling; evidence = CONTEXT.md,
  ADRs, seeds, code, guide) for EVERY design/mechanism/standing-rule
  choice — not only pivotal ones. The grilling format is binding: rounds
  over the design tree, the whole frontier per round, numbered questions
  each with a recommended answer, wait for answers, recompute the
  frontier; domain-modeling sharpens terms against GLOSSARY and writes
  ADR/glossary entries as decisions crystallize.
  No tacit decisions: an ungrilled fork that ships is a finding. The
  grill names the alternatives and picks the optimal path WITH the user;
  the chosen path and its rejected alternatives land in the seed/ADR.
  Rounds are DELIVERED through the `question` tool — one call per round,
  the options carry one-line descriptions WITH the Jev pre-screen
  distribution, the recommended option listed first and marked
  "(Recommended)", free-form answers through the auto-added own-answer
  option (user directive 2026-10-26: structured selections instead of
  free-text question lists in chat; the answered record is what gets
  written into the design). The noul dimensions are their own question
  in the same call.
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
  EXTENSION (user directive 2026-10-27, sprint 58): until the
  re-armed line is PROVEN reliable (work items verifiably worked by
  runs, the #990 supervised laps green), propose NO lane moves and NO
  line assignments at all — not even as recommendations; the session
  works every work item locally in sprint order, and the cycle
  report's ASSIGNMENT PENDING section recommends LOCAL work (next
  sprint candidates) instead of `@fabro` assignments. Lane assignment
  resumes only on an explicit user decision after reliability is
  proven.
  VET BEFORE WORKING (user directive 2026-10-27 evening, born in the
  #990 laps): every item considered for work — lap, lane, or local
  sprint — gets a RELEVANCE TEST against the CURRENT codebase first:
  does the cited path/mechanism still exist, is the demand overtaken
  by landed work? Migration-era items are presumed stale until vetted
  (median open age ~50d; the laps caught two such artifacts live: the
  project-facts lane bug and a zombie doc item #347). A vetting pass
  over the lane pools is standing backlog work, not ceremony.
  LANE ADMISSION (user directive 2026-10-29, extending the test):
  an item enters a lane ONLY after that check passes for THAT item,
  individually — never bulk-assign the migration-era pool, whose
  premises most restructures have overtaken. Unvetted pool entries
  are how a re-armed picker burns runs on dead premises. The backlog
  relevance pass executes this: close the dead items with evidence,
  keep only the hand-checked, high-quality ones.

## Phase 2 — Build (local-first; delegation only by explicit choice)

- File the seed with the agreed design BEFORE any code exists — design
  decisions live in the tracker, never only in chat or the diff.
- LOCAL builds follow the sprint discipline: claim
  (`seeds update --status in_progress`), implement under the mechanical
  gates of the touched surface (Rust: rust-style-guide + guideline pages
  loaded and NAMED before the first edit cell), verify with real
  commands, close with evidence.
- Pin power is PROVEN, not assumed (2026-10-03, fabro-8615): after a
  regression pin is written, restore the PRE-FIX implementation (the
  file as it stands in HEAD) and confirm the pin goes RED, then restore.
  A "nearby" mutation of the new code can pass while the pin is blind -
  two variant mutations of the new scanner survived, only the HEAD
  restore failed the run-level pin with the real symptom
  (`Failed { reason: SoftStop }`). A test added through an EXISTING
  harness must prove the input reaches the seam: the projection harness
  never wrote the run spec's `graph_source`, so the first engine-level
  pin was trivially green until the harness carried it. A sed-both-ways
  mutation restore must verify the pattern is UNIQUE in the file first
  (grep -c == 1, or anchor on surrounding context): the sprint-54
  reverse sed rewrote `#[arg(long, value_enum)]` in THREE sibling
  commands while restoring one and reddened unrelated worker_auth ITs
  (caught only because the test filter matched them). Two sprint-60
  (2026-10-28) pin traps extend this: (a) a bare nu block `{ ... }` as
  a statement is a CLOSURE VALUE, never executed — a pin arm written
  that way reads as green while it never ran (the first closeout
  parse-pin was doubly blind: unexecuted AND a hardcoded argv copy);
  executable arms need `do { ... }`. (b) A lenient shim (nu parses
  declared flags wherever they appear) is POSITION-BLIND and can never
  pin an argv grammar — pin CLI invocation shapes by driving the real
  script END TO END with the REAL binary against a dead server and
  asserting the failure is transport, not a clap parse error (the
  work-create `--project`-before-the-verb loss, run 01M60NX9W7N6).
- A NEW-flag IT failing with clap's "unexpected argument" is a FLAG-NAME
  mismatch until proven otherwise (2026-10-26, work#995 handoff): clap
  derives `--<snake_case_field>` when no explicit `long =` is set, so a
  field `item_type` ships `--item-type` while tests, docs, and the
  agreed surface say `--type`. Compare the bare field name against the
  agreed flag BEFORE suspecting the test harness or a stale binary —
  and a pending snapshot for a NEW feature whose captured output is the
  FAILURE itself (error text, nonzero exit) means `cargo insta accept`
  baked the bug in: rewrite the snapshot to the intended behavior, never
  accept a failure-shaped snapshot of a new feature.
- Run the touched tests IMMEDIATELY after each refactor edit, not at the
  end of the batch: three self-inflicted scanner bugs (two byte offsets,
  chain legs) each cost a hung 120s/20s test round; the unit test
  catches them in 0.02s. Slice arithmetic is the recurring trap: never
  pass an index computed against the FULL text into a walk that slices
  a SUFFIX (fabro-e901 hung every envelope test this way) - carry the
  absolute cursor beside the relative offset. The full direct-fix recipe below
  (lint first, dry-runs, unpiped exit codes) is binding for local work.
- Delegated builds (the explicit-choice case): feed implementation seeds
  with pointers — files, trait seams, guideline pages.
- Fork wiring (2026-09-30, 3fce cycle): a fix on a forked dep repo is NOT
  shipped until every consuming workspace's Cargo.lock pins the new rev -
  `branch =` patch pins move only on `cargo update`; bump the lock in the
  SAME change (a fabro lock still at the pre-fix petri rev nearly shipped a
  half fix and probe-15 red). Patch BOTH crates of a multi-crate fork repo
  (pebble-coding-agent AND pebble-agent) - a single entry splits shared
  traits into two copies and fails unification. SINCE ADR-0034
  (2026-10-24) petri and pebble are VENDORED in-tree: the fix lives in
  `vendor/<name>`, the `[patch]` block resolves it by path, and there is
  no lock rev to bump - the vendor tree IS the pin (`vendor/PINS.toml`,
  `just vendor-pins`), and the tree is exported to the fork branch in the
  same change. The branch-pin/lock-bump half of this rule still governs
  the remaining git deps (lithos-llm, sandbox-driver, twins).
  DEPS ARE FINISHED ONLY WHEN COMMITTED AND PUSHED (fabro-e71b park,
  2026-10-07): fork-dep work left uncommitted or unpushed at session end
  strands every other machine that needs it - machine 1 had to park the
  e71b takeover because machine 2's petri changes never left the local
  tree. See the dependency-finish standing rule.
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
- Multi-agent limit of the rule above: `cargo clippy` lints EVERY workspace
  member (RUSTC_WORKSPACE_WRAPPER applies to path deps too), so a sibling
  agent's uncommitted red crate blocks/masks your OWN lint verification
  locally (2026-10-03, PR #378: two lints of mine stayed invisible until
  CI). Then: message the sibling the exact lint + fix, fix yours by
  reading, and accept CI as the arbiter for the round — say so in the
  commit message.
- Direct-fix verification recipe (before any push): lint first
  (`just lint-nu` for nu), then DRY-RUNS — positive AND negative — each
  as its own shell call, exit codes read from the process (never behind
  a pipe: `$?` reads the last pipeline member, not the script). Nu
  interpolated strings treat `word:` before a `(` as a command call —
  reword such phrases; parse-clean does not mean run-clean.
- Rust edit cells that INSERT an item before a documented function must
  anchor on the item ABOVE (or the previous item's closing brace), never
  on the target's `#[must_use]`/`fn` line: the doc comment sits above
  that line, and the insert lands BETWEEN doc and fn — the new item
  hijacks the doc and the old one goes undocumented (fabro-091b sprint 28
  review must-fix; rustdoc-only damage, fmt/clippy stay green).
- Rust line-continuations written THROUGH Python (edit skill, insert
  cells) MUST use raw strings (r'''...''') or doubled backslashes: a
  single `\` before the newline is a PYTHON continuation that glues the
  next line's indentation INTO the literal as mid-sentence spaces (f93b:
  corrupted the extracted no-ready diagnostic twice — the second 'fix'
  re-broke it the same way; fmt/clippy stay green, only a full-text pin
  or a reviewer catches it). Pin full user-facing refusal strings in
  tests, not just their prefixes.
- Loop assets are deterministic-script-first: a prompt clause requiring
  judgment over mechanical data becomes a script that prints a verdict
  (`.fabro/scripts/`, `just lint-nu` for new nu scripts; prompts keep only
  the call + verdict routing). Never write and execute an edited script in
  one shell call.
- WORKFLOW CHANGES ARE BENCH-TESTABLE (user directive 2026-10-28): every
  `.fabro/workflows/**` change lands with a bench surface in the fabro-test
  repo that exercises it — a dedicated probe/scenario, or an existing row
  that provably drives the changed asset. The cycle report names the
  surface; a change without one files the fabro-test follow-up in the same
  session.
- BENCH PHILOSOPHY: TARGETED SINGLE-ASPECT PROBES THAT COMPOSE (user
  directive 2026-10-28): the bench tests targeted single aspects in fixture
  twins — the real lane graphs are deliberately NOT bench material (they
  need work items + LLM stages). Compose aspects rather than replaying
  whole lanes: a graph shape against its fold, an environment property
  against the arms that depend on it, an invocation grammar against the
  runtime CLI. And NEVER rerun the full bench for one facet's red: the
  single-probe mode is the default; a red facet gets its targeted probe
  plus, for integration legs, a direct manual drive of the leg's script.
- BENCH CROSS-CHECK VIA PLATFORM LOGS (user directive 2026-10-28): after
  every test round — bench, probe, or supervised lap — query the log
  platform for the window's WARN/ERROR and reconcile the patterns against
  the green table. Green tests are only one side of the coin; the log side
  names what the asserts never saw. Query-mechanics details (service
  filters, auth quirks) live in mulch and the rootprint skill, not here.
- RE-ARM IS A RELIABILITY CHAIN, NEVER A DEFAULT (user directive
  2026-10-29): a line re-arm (scheduler/automation re-enable) is the
  user's explicit call, and it follows the same chain every time — the
  named blocker items are landed, ONE manual supervised lap per lane has
  run green (each log-cross-checked), and only then the user decides. The
  concrete item list of the moment lives in the tracker/handoff, never in
  this skill.
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
  a patch file first; both axes review the frozen snapshot. ONE review
  per sprint, which batches 2-3 items (2026-10-29): it covers the
  batch's whole accumulated diff since the previous sprint's tip, not
  each item separately — per-item discipline (language gate, batteries,
  verification commands) still runs at every close.
- Language gate (ADR-0033): every reviewed artifact is English — docs,
  comments, workflow prompts, seed notes, commit messages, PR bodies. A
  German paragraph or a mixed-language file in the diff is a FINDING,
  not a nit: fix it in the same cycle, translating the whole file at
  once. Only test data that deliberately probes non-English model output
  stays non-English and is named as such.
- Proof of work: the cycle report NAMES the guideline pages loaded for
  the diff and the verification commands actually run.
- Fork-feature regression check: run the fork-only presence suites
  on the reviewed tree — for fabro-workflow the current files are
  `--test fork_ask_tool_registry --test fork_publish_gate --test
  fork_run_tool_pins` (the old `-- fork_seam` filter matches NOTHING
  since the suite split; a filter that runs 0 tests is a silent skip and
  nextest exits 4), plus every newer `fork_*` test file of the crate you
  touched. Every NEW fork feature carries a presence
  pin (fork-only test file + touchpoints row); a pin-less fork feature
  ships only with a filed seed. Red fork-only test = landed fork feature
  regressed: fix or revert, never relax the test.
- Landed prompt diffs get a FACT-CHECK against repo reality: claimed
  branches, paths, command behavior. Prompt hygiene: workflow prompts
  land WITHOUT seed-id literals, run ids, PR numbers, commit shas, dated
  cost narratives, or machine-specific paths (`.fabro/workflows/**`);
  branch/merge facts belong in PROJECT_FACTS. The mechanical net is the
  prompt-lint evidence ban.
- Loop-asset battery sweep BEFORE declaring a gate healthy: run
  EVERY battery named in `scripts/qualitygate.nu` (grep its `smokes` +
  `batteries` lists, run each with `nu`, read each exit code). Cargo
  rounds and the workflow smokes can be fully green while a registered
  parity battery stays red: sprint 23 shipped a fork feature with its
  pin file but no touchpoints registry row, and nothing noticed for
  days (2026-10-08, found by `touchpoints-parity-fixtures.nu`). Seconds
  of work; it is the only net for the missing-row / missing-registration
  class.
- Local full-crate verification runs with `--profile ci` timeouts: a
  default-profile timeout on a Docker/sandbox test under dev-host
  conditions is an environment artifact (known class), not a regression —
  rerun isolated with the ci profile before treating it red.
- After an external interruption (server restart mid-build, user disk
  cleanup): a corrupt target cache shows as compile errors on untouched
  deps (`could not compile syn ... No such file or directory` — fix with
  `cargo clean -p <dep>`) and deleted sandbox plugins as instant
  `PETRI_SANDBOX_HOST_PLUGIN names the host sandbox plugin` expect-panics
  across whole blackbox suites (fix with petri's `mise run plugins:build`;
  fabro tests instead resolve the plugin from PATH). Repair the
  environment BEFORE reading any of it as regression (2026-10-23: 160
  "failed" petri tests were the wiped plugins; one clean -p syn + one
  plugins:build made 1776/1776 green).
- A dep-rev bump under suspicion gets a BASELINE RUN, not a revert:
  restore Cargo.lock to HEAD, move a new pin file aside when it cannot
  compile against the old revs, run the same suite — identical failure
  SET means pre-existing (seed it with the evidence, keep the bump); a
  GROWN set means the bump owns it (2026-10-23: 21 fabro-petri reds
  proven environmental by exactly this isolation, filed fabro-4618
  while the bump landed unchanged).
- fabro-server's IT suite (`tests/it`, incl. the OpenAPI conformance
  test) requires `--features test-support` and is NOT built by a plain
  `cargo nextest run -p fabro-server` — the run stays green while the
  it target silently sits out (2026-10-21: an interrupted unit's missing
  FakeTracker trait impl was invisible until the explicit target ran;
  count the tests against the handoff's expected number). Verify with
  `cargo nextest run -p fabro-server --test it --features test-support
  --profile ci`.
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

- Commit code (plain `git add <files>` + `git commit -F /tmp/msg.txt`)
  BEFORE tracker mutations land. The commit message travels through a
  FILE, never through a Python variable: `bash()` does not see the
  REPL's names, so `-m "$MSG"` commits an empty message (2026-10-03
  fabro-8615) — write `/tmp/msg.txt`, commit with `-F`, then read the
  message back (`git log -1 --format=%B`) before pushing. Stage NAMED
  files only (never `git add -A`) so a sibling session's uncommitted
  work cannot ride the commit.
- Language gate before commit (ADR-0033): re-read the staged diff — every
  written line is English (comments, prompts, docs, seed notes, the
  commit message, the PR body). Cheap mechanical catch:
  `git diff --cached | rg -n 'ä|ö|ü|ß|Zweck|Regeln|Facette|Pflege|wird|nicht|muss'`.
  Hits are inspected, not silently kept; test data that deliberately
  exercises German model output is the named exemption. A sibling repo
  with its own landing dance (fabro-test: `nu scripts/land.nu`) needs
  the same look at its docs, not only at its scripts.
- A SHARED checkout means shared FILES (observed 2026-10-03 with two
  iterate sessions): whole-file commits can sweep a sibling's tracker
  lines into your commit. Check the per-id diff
  (`git diff -- .seeds/issues.jsonl`) for foreign ids before committing
  a shared file, land your own lines promptly, and re-read
  `iterate-state.json` before writing — the sibling session counts its
  closures into the same ledger. CONFIG TRAP (observed 2026-10-06, GB
  era): a hand-edited INVALID `vcs_manager` value made sync wipe
  `.seeds/config.yaml` to an EMPTY file; the fabro repo now carries NO
  `vcs_manager` key (plain default). Valid keys when one is ever needed:
  project/version/max_plan_depth/vcs_manager.
- Shell-written tracker text NEVER carries backticks or `$(` through a
  `bash()` command line (2026-10-08): bash executes them as command
  substitution. A close reason containing `start -> env_guard ->
  tracker_guard` ran the command `start` and REDIRECTED its stdout into
  files named after the remaining words — three empty files appeared at
  the repo root as untracked `A` entries in the status view, and the
  stored close reason silently lost every backticked phrase (the earlier
  record of the sprint had to be rewritten). Write the payload to a file
  and pass `--reason "$(cat /tmp/reason.txt)"`; after any shell-based
  tracker write, sweep `git status` for stray untracked entries and
  delete them before committing — otherwise they ride the next
  `git add`.
- After a MECHANICAL MULTI-FILE REWRITE (sed across a reference path,
  rename wave), derive the staging set from the change itself
  (`git diff --name-only`), not from a hand-typed list, and gate the
  push on an EMPTY `git status` — 2026-10-26 (ADR move to docs/adr):
  two consecutive pushes left riders behind because the sed batch had
  six files and the hand-typed `git add` list had four; the dirty
  remainder was visible in `git status` both times and got pushed past.
  A clean tree is part of the push gate's check, not an afterthought.
  And NEVER a file-WRITE and its file-CONSUME in one PARALLEL tool
  block (2026-10-26, three times: the cp backup raced its own edit;
  the #1003 close cat'd a file that landed a second later and CLOSED
  THE ITEM WITH AN EMPTY REASON — repaired by a gh comment, but the
  close comment is the evidence record). Sequence them: write, observe
  the result, then consume.
  And NEVER amend a PUSHED commit: the branch refuses force-push and
  local/remote diverge (2026-10-26, the rotation rider: amend hit an
  already-pushed sha; recovery is `git reset --soft <pushed-sha>` +
  a fresh commit — correctable, but a wasted gate round). Check
  `git status` against the remote tip before any amend.
- A `fabro create` whose post-processing dies still created the run:
  capture exactly one run id per intended create and `fabro rm --force`
  duplicates immediately - submitted ghosts count as active runs and wedge
  the push gate. The SAME class hits `seeds create` (2026-10-05, sprint 19:
  d7e7+3523 duplicated f93b+c167): piping a create's JSON into a consumer
  that crashes still lands the record - write CLI json to a FILE, parse the
  file, and sweep the tracker diff for surprise new ids before committing.
- Push policy: during the cycle, branch updates and read-only integration
  stay allowed; the PUSH direction is gated to one mechanical decision at
  the END: `nu .fabro/scripts/push-gate.nu` (exit 0 = open: no running
  conductor/develop/revisor pass AND no open run PR). Check and push
  share one cell, the gate runs UNPIPED, and the push itself is
  `git push origin denkhaus` — `&&` behind a pipe reads the pipe member's exit
  code, not the gate's (2026-10-02 23:00 incident: a
  `gate | tail -1 && git push` pushed straight through a REFUSED verdict
  while a loop pass ran), and `cmd; echo RC=$?; if [ $? -eq 0 ]` reads
  ECHO's status, not cmd's — capture `rc=$?` on the line directly after
  the gated command and branch on `$rc` (2026-10-03 near-miss, arch-branch
  push cell). A repaired gate is validated against a known-active
  line state before its first OPEN verdict is trusted. Incident restore
  may push as soon as no pass runs. Evidence and history: `ml prime git`.
- Deploy windows: deploy only while no conductor pass runs. PRE-DEPLOY
  BENCH GATE (fabro-6041 grill 2026-10-24, user GO): before EVERY deploy
  window, run the fabro-test workbench — `cd ~/dev/fabro-test && just up
  && just workbench` (refreshes the local image, then the facet table:
  probes + platform checks + mini, ~15 min, <1 EUR). A RED table means
  NO window: fix or seed first (the repo's own rule — new product bugs
  land as FACETS row + probe BEFORE any deploy). Extends the repo's
  'after every upstream merge' cadence to fork releases. GATE-TOUCHING
  DEPLOYS (fabro-2919, Grundsatz-review 2026-10-12): before the window,
  run `just smoke-window-gate` (the twin-scenario smoke: scripted 429 ->
  tick holds + manual fire refused; reopen -> next recheck tick fires) —
  a gate fix that needs a deploy to find out is a 40-minute turnaround;
  the smoke answers it in seconds, locally. Pause the
  line first (automation replace with FULL body + `If-Match` revision +
  explicit `on_overlap: skip`; re-GET and verify it survived), deploy
  nonblocking, smoke on mirtuell.net, re-enable, monitor via heartbeat.
  POST-DEPLOY PROOF CEREMONY (2026-10-12, fork.14 lesson): a deploy is only
  'done & working' after ONE live proof of the shipped behavior — for
  gate/automation changes fire the automation once (between ticks,
  on_overlap-safe) and observe the run starting; never report success from
  version strings alone (the 17:30 Berlin tick ran 13 min BEFORE the 17:43
  apply and faked a 'fix does not work'). FAST DEPLOY RECIPE (same lesson):
  warm gopass BEFORE the build, edit the tofu pin DURING the build, skip
  CLI install + prune when unchanged — mechanical target <=30 min from
  commit to live proof.
- Tooling-repin rule (user directive 2026-10-04, "simple but
  consistent"; GB era ended 2026-10-09): a GitButler-fork release repin
  is COMMITTED locally and NEVER pushed on its own — the session owns
  every line push (behind the gate). Repins riding their own push bypass
  the push gate and move the line tip past the deployed sha (fork.5
  incident: parity refusal + stale release clone). Deploy-window ORDER:
  repin (if pending) -> version bump -> push -> clone-build -> tofu
  apply -> pin-toolchain — one window restores `line tip == deployed
  sha == pin target`.
- Fork releases follow ADR-0025 naming `<frozen-base>-fork.N` — the
  upstream `cargo dev release` path (origin main push) is WRONG for the
  line branch. Until a fork mode exists: manual bump = workspace
  `Cargo.toml` version + `cargo update --workspace` + `just
  image-release` (tag `<version>-<sha>`); N increments once per built
  release; the base moves only with a deliberate intake decision.
- Secret-gated steps (tofu/gopass) are probed with a BLOCKING
  `gopass show` WITHOUT a timeout wrapper BEFORE the step that needs
  them (user directive 2026-10-15: a `timeout`-wrapped probe KILLS the
  user's unlock prompt mid-entry — the mask disappears as soon as they
  start typing; the probe must wait for the unlock, not race it); the
  store relocks on its own TTL — ask the user to warm it EARLY, not at
  deploy time. Recompute the sandbox-plugin sha per deploy (same pin rev
  usually means the same sha — verify, never assume).
- Production deploy after substantial engine changes (binary-need check:
  any `lib/` path in the merged work): `just image-release` +
  fabro-tofu apply — NON-INTERACTIVE (the justfile profile picker is
  fzf-only by design; name the profile explicitly instead): cd
  ~/dev/fabro-tofu && export TF_VAR_state_passphrase="$(mise exec --
  gopass show -o fabro-tofu/state-encryption)" && TF_DATA_DIR=
  .terraform-prod mise exec -- tofu plan -var-file=envs/prod.tfvars —
  apply identically with `apply -auto-approve` after reviewing the
  3/0/3-style plan. `just up`
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
   to mulch (`ml record <domain> '<content>' --type <failure|convention|
   pattern|decision|reference|guide>` — type is required, failures also
   need `--description` and `--resolution`; then `ml sync` — real
   insights only); durable preferences to memory. No finding stays
   unrecorded. A skill edit carries the ABSTRACT RULE only (user
   directive 2026-10-29): session state, live item numbers, and incident
   debugging detail belong in the ledger note, the tracker, or mulch —
   never here. A skill full of stale session specifics erodes trust in
   every other line (this file describes the dev cycle, generally).
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
  (user directives 2026-10-02 evening, amended 2026-10-29): a SPRINT is
  one batch of 2-3 work items (Phase 1 composition) and counts ONCE when
  it closes at least one item this session worked LOCALLY with a
  substantive diff — the batch's other closures ride the same sprint, they
  do not each increment the counter. Docs-only, row-only, and verify-only
  closures never increment. LINE closures (loop lane, delegated
  develop runs) NEVER count — the line is not inside a sprint. Historical
  note: the bootstrap-era count of 3 includes develop-lane closures under
  the superseded lane-blind rule; kept as history, not recounted.
  Notes live in the `sprints` field (user directives 2026-10-26): an
  array of `{"sprint": <n>, "notes": ["<text>", …]}` records — ONE
  record per sprint, SEVERAL notes per sprint possible (corrections and
  gate passes ride alongside the sprint note), further fields get
  added per need, and the record doubles as its own JSONL row.
  Notes rotation (brightctl port, retro 2026-10-17): after appending the
  sprint note, run `just rotate-notes` when sprints > 10 — the state file
  keeps the newest 10 SPRINTS, older ones move UNCHANGED as one-`{"sprint":
  …, "notes": […]}`-record-per line JSONL into `iterate-state-archive.jsonl`
  (history stays git-diffable; both ends validated against the record
  shape; dedup keys on the sprint number). Deterministic and idempotent;
  `just rotate-notes-dry`
  previews. 90 notes cost every session start the 50 KB read-tool limit
  (134 KB file). The script (scripts/rotate-sprint-notes.nu) is
  SESSION-OWNED: never registered in qualitygate batteries, never called
  by lanes, absent from the reflection query — it changes the file, not
  the loop.
5b. Heartbeat cadence tracks reality (user directive 2026-10-18: OFF
  while idle): 20m while anything autonomous runs (pass, build, deploy
  chain); PAUSED (not 60m) when everything is user-gated or nothing runs.
  Do NOT recreate the line-watch heartbeat at cycle start as ceremony —
  create/resume it only when autonomous work is actually in flight; the
  user's pause is authoritative and never overridden. Refresh the
  instruction at every phase change — a heartbeat still carrying a
  completed critical path is noise that erodes trust.
6. Line-watch heartbeat (label `line-watch`, interval 10m, follow-up
   delivery): pull, evaluate journals/reviews (premise-checked against
   the tree), dispatch seeds per ADR-0018 with dispatch-dedupe (keep the
   richer seed, close the lesser naming both ids), push through the gate
   (`git push origin denkhaus`, gate unpiped) with JSONL-dedupe discipline, rootprint correlation for every
   evaluated run, report compactly in German.
   SALVAGE SWEEP is MECHANICAL FIRST (fabro-f312, 2026-10-04): run
   `nu .fabro/scripts/salvage-sweep.nu` (optionally `--since 48hr
   --max-runs 25`) and route its verdicts — `salvage:filed` means the
   sweep already filed the pointer seed, `salvage:none` /
   `salvage:pointer-exists` mean nothing to do, `sweep:skip:<reason>` is
   journaled verbatim. The SESSION owns this call because it holds the
   stored server login; a run sandbox has none (a lane-side consumer must
   go through the server or the engine's run tools, never through
   `fabro ps`/`fabro dump` inside a sandbox). RLM heartbeats are
   session-scoped: recreate from this spec when missing; if the session
   host rejects heartbeats entirely, ask the user to set the visible
   /heartbeat with this ceremony — never run silently unwatched.

## Standing rules

- COMPLEXITY BRAKE (ADR-0028 lesson, user Grundsatz-review 2026-10-12): a SECOND
  patch on the same mechanism within days is a smell — the mandatory response
  is simplify-or-REVERT the mechanism (cite the ADR that owns it), never a
  third patch. Patches on engine gate behavior cite the owning ADR; a patch
  that contradicts a standing decision (even a code-comment one like
  2026-09-14's recheck design) stops for an explicit supersession, never a
  silent additive layer.
- WRITTEN LANGUAGE IS ENGLISH (ADR-0033, user directive 2026-10-24):
  chat German, artifacts English — in EVERY repo of the line (fabro,
  fabro-test, fabro-tofu): docs, ADRs, catalogs, code comments, workflow
  prompts, seed notes, commit messages, PR bodies, review notes. A
  German file never licenses German additions; translate the whole file
  in the same change. Only test data that deliberately exercises
  non-English model output stays non-English, and it is named as such.
  Distinct from NO LANGUAGE SPRAWL below: that rule is about SCRIPT
  languages (nu vs python vs sh); this one is about the written language.
- NO LANGUAGE SPRAWL (user top priority, directive 2026-10-03): every script invoked by workflows, justfile, or gate/tooling entry points MUST be a nu script. Never add, extend, or split non-nu script assets. The one standing violation — the upstream code-review lane's 16-file / 8.5k-line python engine — was RETIRED on 2026-10-08 (user GO) as never used in this fork instead of ported (fabro-c16a closed; the graph, rules, templates and pins are gone, the upstream asset stays recoverable from upstream history). POSIX sh survives only where the failure mode it guards is a sandbox WITHOUT nu (`.fabro/scripts/toolchain-guard.sh`). Architecture scans and sprint plans check the language axis BEFORE proposing splits/refactors.
- Style-guide pair invariant (fabro-6538, 591a decision 2026-10-03):
  `.fabro/skills/rust-style-guide/` is canonical, `.agents/skills/rust-style-guide/`
  its synced mirror. After ANY edit to the canonical side, the session runs
  `just sync-style-guide` (the qualitygate parity battery REDs on divergence).
  Loop runs NEVER write `.agents/**` (fabro-591a decision: no run-scope
  widening — battery-RED routes the sync to the session).
- VCS layer is PLAIN GIT (user decision 2026-10-09: the GitButler
  experiment ENDED after ~14-19h friction over 6 days, zero current
  multi-agent use, and a permanent sha tax; the fork stays frozen at
  v0.22.3-fork.7). Normal git discipline: named-file staging, commit -F,
  ff-only pulls, pushes only behind the gate. HISTORY (2026-10-03..
  2026-10-09, GB era): every write went through `but`; target was
  `origin/main` (frozen ADR-0024 base) with the line as the applied
  virtual branch `denkhaus`; a stale git index under GB made
  `git diff`/`git status` show phantom `MM` entries (proof of real
  change was `git diff HEAD -- <files>`). Machine 1's teardown ran
  2026-10-16; every machine is plain git now (a checkout still on
  `gitbutler/workspace` is a teardown gap — see Phase 0).
  Do NOT merge origin/main (upstream intake stays frozen); line updates
  via `git pull --ff-only origin denkhaus`, line pushes via
  `git push origin denkhaus` behind the gate. The stale
  `origin/gitbutler/workspace` shadow branch (2026-10-03 snapshot) was
  verified gone from the remote 2026-10-16 — nothing left to clean.
- Wait budgets derive from observed service latencies, not guesses: a
  bounded wait must outlive the slowest LEGITIMATE stage of what it waits
  for (PR creation after terminal status; required-check duration on a
  PR). A fallback that bypasses a verification path must re-check the
  gated path one last time before engaging, and clean up the ghost it
  creates (open PR, remote branch) so gates downstream inherit nothing.
- Judgment pre-screens (ADR-0022, session-side INTEGRATION — user directive
  2026-10-16: use it everywhere or never get evaluation data): advisory,
  fail-open, via `.fabro/scripts/judgment.nu` (criteria is an option-id ->
  description RECORD, never a bare array); every call logs to
  `~/.local/state/fabro-judgments/<date>.jsonl` (skill/phase/subject
  provenance) — the log IS the fabro-d4c6 evaluation corpus, so a phase
  that CAN pre-screen and does not is a finding. Application points:
  (a) EVERY grill-with-docs fires a pre-screen BEFORE the user grill —
  choice questions per fork (options carry one-line descriptions) plus
  noul for risk-appetite dimensions; present distributions WITH the
  alternatives; noul answers stay user-owned; record the run in the
  seed/decision (framing-bias caveat named); (b) cycle review: verdict
  pre-screen per evaluated run (approved/changes_requested/
  verification_blocked) and per-file residue adjudication, compared
  against the reviewer's own verdict — mismatches are findings;
  (c) line-watch journal routing: painpoint severity as noul
  (seed-worthy or not) before dispatching; (d) salvage verdicts:
  noteworthy-or-not choice against the sweep's call; (e) PR triage when
  a merger lane exists (parked in fabro-c14e's demand). Thresholds still
  only after the d4c6 report; until then the answers advise, the
  documented user decision decides.
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
- Upstream posture (ADR-0024 + ADR-0027): the platform base is FROZEN at
  0.362.0-nightly (merge-base 1b4fb1528); since ADR-0027 (2026-10-09) the
  ENGINE DEPENDENCIES are frozen as well — no routine dep intake, no
  routine fork merges of petri/pebble/lithos-llm/sandbox-driver/twins.
  The decisive finding behind that: three read-only inventories showed
  denkhaus-0 to have all ten capability axes we rely on and ~74% of the
  Petri-era closures to be REBUILD/FIX of pre-existing capability, while
  the host-hook gap we were chasing exists in BOTH worlds (containerized
  server, no host checkout) — so the archived line was no fix for it.
  What a return would have LOST: the record/projection run stack (server,
  CLI, SPA, DB) and the ~24% genuinely new machinery. A dep bump is now a
  deliberate decision with a recorded reason; drift watching stays
  informational; still selectively OFFER general, non-strategic fixes
  upstream (fabro-d485), strategic assets stay fork-private. Keep the
  .fabro layer portable (seams/presence pins stay binding). Re-open
  condition (ADR-0027): the engine structurally blocks a change we cannot
  make in the fork.
- Dependency finish (user directive 2026-10-07, fabro-e71b park): a
  session that lands work in a fork dep FINISHES it before it ends, and
  since ADR-0034 (2026-10-24) the two engine forks are IN-TREE:
  petri/pebble live at `vendor/petri` and `vendor/pebble` (snapshots,
  path patches, provenance in `vendor/PINS.toml`). The finish rule
  becomes: the vendored edit and the consuming change are ONE commit,
  and the tree is EXPORTED to the fork branch in the same change
  (`just vendor export <name> --message '…'`), so history and the
  upstream-PR vehicle stay alive; if an export is impossible
  (credentials, network), the handoff seed names fork, branch and local
  state EXPLICITLY — dependency work never ends silently unfinished.
  Session-close check for every vendored change: `just vendor status`
  clean (tree matches PINS, fork branch carries the sha) and
  `just vendor-pins` green (registered in battery-runner). Uncommitted
  or unexported engine changes park the next machine exactly like the
  old external checkouts did (machine 1 could not take over fabro-e71b
  because machine 2's petri changes sat uncommitted at session end).
  Upstream merges stay FROZEN, and since ADR-0027 (2026-10-09) the
  DEPENDENCY BUMPS are frozen too: lithos-llm/sandbox-driver/twins are
  pinned by lock revs and `.fabro/scripts/dep-pins-fixtures.nu` REDs
  when a locked rev moves; petri/pebble are pinned by the vendor
  snapshot and `scripts/vendor-pins-fixtures.nu` REDs when the tree
  drifts from PINS or a patched crate resolves from git again. A dep
  bump is a deliberate decision: reason + the pin edit in the same
  change. Patching our forks stays allowed and expected.
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
- HANDOFFS ARE TRANSPORT ARTIFACTS, NOT HISTORY (ADR-0036, user
  directive 2026-10-26): a handoff exists to carry tasks from one
  session to the next. The moment its demands are consumed (landed or
  filed as work items), the handoff is DELETED — a markdown handoff
  file is `git rm`ed in the consuming session (git keeps the history;
  no superseded banners, no doc-graveyard), a work-item handoff is
  CLOSED with the evidence. The consuming session also cleans
  references to the deleted file (checklists, docs) in the same change;
  ADR citations stay untouched (historical record). Leaving a consumed
  handoff in the tree is a finding of the same class as stale docs:
  the next session reads it as current state.
