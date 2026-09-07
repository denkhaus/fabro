---
name: iterate
description: Fabro dev lifecycle in one flow. Use when the user invokes /iterate or asks to start or continue a development cycle - select seed work, build, run code-review, deepen with improve-codebase-architecture when warranted, grill with docs at pivotal forks, and close EVERY cycle with the mandatory self-reflection step that optimizes this skill, records learnings to mulch, and files seeds for new demands. The rust-style-guide skill is the binding Rust coding policy in every phase.
---

# /iterate (fabro only)

One development cycle, end to end. The user starts a cycle by invoking
/iterate; the agent drives everything else. Chat replies in German,
all written artifacts in English.

## Coding policy (Rust)

The `rust-style-guide` skill is THE binding Rust coding policy for all
Rust work in this repo - upstream code, our platform changes, and local
features share ONE style. It is the same guide the upstream workflow
agents use (supporting files in `.fabro/skills/rust-style-guide/`). It is
central in every phase below: grill against it, plan against it, build
with it, review by it, deepen with it. A deviation from the guide is a
decision, not an accident - it needs the user plus an ADR.

## Phase 0 - Orient (always, cheap)

- Which world am I in? `git branch --show-current`. Since the world
  merger (d9138572c, ADR-0013) platform AND product work both live on
  `denkhaus`; `denkhaus-lab` is legacy (retirement pending, multi-context
  see docs/agents/domain.md). NEVER git worktrees - branch switches
  happen in the main checkout.
- Is a workflow cycle in flight? (`fabro ps`, or a `just run`/`just
  cycle` process). Serialization principle (ADR-0015): while the
  develop/revisor workflow works the tracker, this agent session does
  NOT claim seeds - one line, one executor. Wait for terminal state or
  ask the user. The line's work queue is the @fabro-assigned view
  (`sd ready --assignee fabro`, ADR-0018): a running pass plus an
  empty queue is fine (fail-closed park between cycles); a parked pass
  with a NON-empty queue, a parallel pass, or a lost on_overlap are
  incidents (see standing rules).
- Interrupted cycle? Reconstruct BEFORE selecting: `git status` (uncommitted
  diff is the interrupted operation) + `sd list --status in_progress` tell
  you what was mid-flight; continue that work instead of picking a new seed.
- `sd ready` and the open-seed list for candidates. `ml prime <domain>`
  only when the cycle touches that domain.

## Phase 1 - Select

- Pick the highest-value open seed, respecting family order
  (stage envelope, ADR-0009: 900e -> 47b5 -> e47c -> e804 -> ba96),
  blockers (`!`), and the user's current focus.
- If the choice or the design is pivotal or unclear: run a
  grill-with-docs session FIRST (grilling + domain-modeling; evidence =
  CONTEXT.md, ADRs, seeds, code, rust-style-guide). Never guess at
  weichenstellende decisions - they belong to the user.
- Public-contract NAMES (graph attributes, API fields, tool names, CLI
  flags) are pivotal too: name the capability, not the consumer's motive
  (2026-08-31 lesson: `revises` -> `inspects`, user decision AFTER
  implementation - cheap only because nothing was committed). Surface
  naming for user decision in Phase 1 or before the first commit,
  never settle it silently in the diff.
- Plan against the guide: the plan names the guideline pages the design
  must satisfy (newtypes, enums-vs-traits, error taxonomy, async task
  lifecycle, ...). A design that must deviate is itself a pivotal fork -
  put it to the user in the grill session, never drift silently.
- Executor decision (user directive 2026-09-06, extends ADR-0013
  dogfooding + ADR-0015): the agent implements NO seeds itself anymore
  - product AND engine/platform seeds alike go to the autonomous line.
  The conductor is cron-activated and drives the develop workflow; it
  picks, claims, and implements seeds on its own schedule. Manual
  `just run develop` / `just cycle` starts are repair actions only,
  when the autonomous line is down, and with the user's knowledge.
  This agent's /iterate job: orient and monitor the autonomous line,
  grill pivotal design forks with the user and write the agreed
  design INTO the seed BEFORE the workflow implements it, review
  landed diffs, revise the autonomous workflows themselves (standing
  directive), reflect, and report. The workflow owns implementation,
  not review or reflection.
- Claim the seed: `sd update <id> --status in_progress` — DIRECT
  implementation only. In delegation mode the goal names the seed and
  the RUN's planner claims it; an agent-side claim would remove it from
  `sd ready` and the planner would find nothing (cycle-2 insight,
  2026-09-05).

## Phase 2 - Build

- Seeds BEFORE implementation (user directive 2026-08-27): file the seed
  or update the existing one with the agreed design BEFORE writing code.
  If implementation goes wrong, the plan must already be durable in the
  tracker — design decisions never live only in chat or in the diff.
  Close/update the seed with evidence after the work lands.
- Rust changes (platform AND local features): load the rust-style-guide
  skill FIRST - `guidelines.md` plus only the pages the task needs, and
  `workflows/new-rust-project.md` when creating or configuring a crate.
  Write code that conforms from the start; do not retrofit style after
  review. MECHANICAL GATE (user directive 2026-09-06, after the wait.rs
  rework round): reading SKILL.md + the TOC is NOT enough - the actual
  guideline PAGES for the diff (async, errors, logging, naming, testing,
  ...) are read in the SAME turn as the design, BEFORE the first Rust
  edit cell, and their names go into the session notes for the cycle
  report. A Rust edit without prior page load is a process defect even
  when tests pass.
- Platform/engine change: ALSO delegated to the autonomous line
  (user directive 2026-09-06) - PR #28 (fabro-tool, b1b6df41f) proved
  the develop workflow ships engine Rust. This agent feeds the seed
  with pointers (files, trait seams, guideline pages) instead of
  writing the code. Direct code edits happen only when the user
  explicitly assigns them to this agent in chat.
- Product/lab change or engine+workflow validation: run the develop
  workflow via the wrapper (`just run <workflow> ...`, or `just cycle`
  for develop+revisor; ADR-0015: the wrapper no longer runs an ask
  review - the revisor owns revisioning); quality gates run inside the run.
- ADR-0008: change graph, prompts, scripts, and settings as ONE unit
  in the same change.
- Inserting an item BEFORE a struct via an anchor on the struct's own
  line detaches the struct's derive/attribute block onto the NEW item
  (fabro-09ea, 2026-09-02: the enum landed between `#[derive(...)]` and
  `pub struct Automation` — duplicate-impl and missing-derive errors).
  When the anchor is an item's declaration line, check the lines ABOVE
  the anchor belong to it, and include or explicitly re-attach them.
- Apply file edits as single-flow assert->write cells (fabro-8d30a,
  2026-09-02): a Python edit cell whose write_text sits in an early-exit
  branch silently drops the edit — the OpenAPI path block and a struct
  field were both lost this way and only the compiler/codegen revealed
  them minutes later. One cell, one write, assert before it; never a
  conditional write path. Verify with a grep of the written anchor
  immediately after the cell.
- Fork-to-edge checklist before the first commit (fabro-7461, 2026-09-02):
  when the user approved a design with numbered forks (loop, fail modes,
  gate routing), re-check EACH fork is literally visible in the artifact
  (an edge, an attribute, a prompt line) — the revisor's approved LOOP
  was missing from the first graph draft; only the spec reviewer caught
  it. Approved-but-invisible forks are the most expensive class of drift
  because everything else looks done.
- English everywhere written; no inline shell logic in the justfile
  (scripts/*.nu only); shell_quote for sandbox command strings.
- After programmatic (slice/regex) edits to large files, verify uniqueness
  of the touched definitions before building: a mis-anchored slice
  duplicated a ~300-line test region in fabro-4556 and only the compiler
  caught it; counting `fn <name>` occurrences is cheaper than
  checkout-restore-and-reapply.
- Adding a struct field across a crate: do NOT regex-sweep struct-literal
  sites by grep — run the test compile (`cargo nextest ... --no-run`) and
  use the compiler's missing-field list as the worklist. A ba96 regex pass
  mis-injected into `impl` blocks and `-> Type {` return positions and
  needed three fix rounds; the compiler enumerates exactly the value
  positions. Same class: when generating Rust string fixtures from Python
  heredocs, print/verify the written literal (or test-compile it) before
  building — escape handling silently produced invalid patch fixtures and
  space-run diagnostics (ba96, 2026-09-03).

## Phase 3 - Review

- Run a code-review skill session on the diff since the base point
  (standards axis + spec axis). Fix findings before continuing.
- Review the FROZEN diff: snapshot `git diff HEAD` (plus untracked files)
  to a patch file and point both subagents at it. Do not start applying
  one axis's findings while the other axis still reviews the live tree —
  a drifting tree makes the spec axis report phantom breakage (e47c
  lesson, 2026-09-02).
- Clean-tree control experiments (stash to prove a failure pre-exists):
  snapshot the working diff to a file FIRST, run the control, then verify
  restoration by comparing `git status` before/after. Never suppress
  `git stash pop` output — a silently dropped pop can lose fixes and
  surface only at final verification (e47c lesson, 2026-09-02).
- Red/green-case SIMULATIONS on throwaway branches are the same class
  (fabro-1dae, 2026-09-02): a `git add -A <scope>` + commit + reset
  --hard cycle on the sim branch silently swept uncommitted review
  fixes into the sim commit and destroyed the unstaged rest. Before any
  simulation: stash the working tree (or commit first), snapshot the
  diff, then cut the sim branch from a CLEAN tree; restore and VERIFY
  byte-identity (the workflow.fabro closing brace has no trailing
  newline — a whole-line `sed` can eat structural bytes).
- PROOF OF WORK (user directive 2026-08-27, after the fabro_run_logs
  cycle skipped the guide): the cycle report NAMES the guideline pages
  loaded for the diff and the verification commands actually run
  (tests/clippy/rustfmt per package). A report without both means the
  review did not happen - a skipped guide load is a process failure,
  even when tests pass (the fabro_run_logs cycle shipped two logic bugs
  - inverted severity ordering, ISO-dash timestamp check - that a
  guide-first pass would have caught at design time).
- The standards axis IS rust-style-guide (plus the AGENTS.md strategy
  docs): reviewers load `workflows/code-review-refactor.md` and the
  guideline pages covering the diff. Guide findings are findings, not
  opinions.
- Verify a reviewer's FACTUAL premise in code before fixing (e804
  lesson, 2026-09-02): the spec axis claimed raw blob:// refs reach the
  tool; the dispatch path had already resolved them to file:// pointers.
  The premise was wrong, but it exposed an adjacent hardening gap that
  WAS real. Read the call path first, then fix what is actually true.
- Clippy failures on UNTOUCHED files after mixing stable builds with the
  pinned nightly can be stale-cache artifacts (e804): snapshot the diff
  (including untracked files - stash ^3 parent carries them) BEFORE a
  clean-tree control, and re-run the lint on the clean tree before
  believing the failure.
- Tool-call misuse by agents is BOUNDARY evidence, not prompt material (user
  review 2026-09-06): when an agent repeatedly drops required fields or
  guesses payload shapes, fix the boundary first — verbose validation errors
  naming property paths (fabro-e4ac) and contract ergonomics that accept the
  natural payload (fabro-528b: workflow_source-only specs). Prompt example
  fixes are stopgaps only; the boundary seed is filed in the same session.
- REVISE THE AUTONOMOUS WORKFLOWS (user directive 2026-09-05, extended
  same day): after every delegated cycle or conductor pass, inspect the
  journals and stage outcomes of the AUTONOMOUS runs themselves —
  revisor, conductor, merge-upstream — because none of them has its own
  revisor; findings that only surface in a run's own journal (tool
  walls, write drops, prompt defects, orchestration gaps) die silently
  otherwise. Every such painpoint must land as a seed, a skill edit, or
  an explicit no-action note in the cycle report - never in chat only.
  A full `fabro ask <run-id>` improve-review is NOT needed every cycle
  but is worth running occasionally (e.g. every 5th cycle or when the
  journals show recurring unresolved painpoints).
- E2E-test verification (2026-08-31 lesson, fabro-47b5): a green
  `--profile e2e` run can be a NO-OP for twin-only tests
  (NEXTEST_PROFILE=e2e => TestMode::Strict => e2e_test(twin) prints
  "skipping" and returns Ok) — run twin tests in the DEFAULT profile.
  And tests/it's `LlmCodergenBackend`/`make_llm_backend` seam never
  builds agent sessions: session-level features need httpmock + the
  REAL `AgentApiBackend` (wire-body asserts are the strongest proof).

## Phase 4 - Deepen (conditional)

- When review surfaced structural smells or the touched area needs
  design sharpening: run improve-codebase-architecture. Skip when the
  cycle was mechanical - not every cycle needs this.
- Judge deepening proposals against the guide as well: enums-vs-traits,
  newtype vs primitives, error taxonomy and layer boundaries, public
  API evolution, module visibility. codebase-design supplies the
  vocabulary (depth, seam); rust-style-guide names the target shape.

## Phase 5 - Integrate

- Commit and push. Deploy/smoke where the domain requires it
  (platform: `just up`; lab: run_workflow integration).
- DEPLOY WINDOWS (user directive 2026-09-07): `just up` ONLY while no
  conductor pass runs. Pause the line first (PUT automation replace
  with schedule enabled:false - PRESERVE on_overlap, replaces and UI
  edits can wipe it), deploy nonblocking, verify smoke (health, ps,
  authenticated automations probe), then re-enable the schedule with
  on_overlap=skip again. A heartbeat (~5m) monitors the deploy and
  performs the resume.
- BINARY-NEED CHECK (2026-09-07 lesson, user caught it): claiming
  'repo-side only, no binary need' for a merged PR requires diffing it
  against lib/ - .fabro/docs/tracker-only changes skip the rebuild, but
  any lib/ path (even a small crate like fabro-validate) changes the
  server/CLI image and needs the next just up.
- PUSH/PR COORDINATION: pushing to denkhaus while a run PR is open can
  turn it DIRTY and stall auto-merge. Check open run PRs before
  pushing; if one goes dirty with a green gate, update its branch
  (union-resolve .seeds/.mulch JSONL conflicts, keep both sides, no
  duplicate ids). reached=blocked (PR #34) surfaces stuck gates in the
  wait, but the branch update itself stays manual until fabro-94e8.
- Close or update seeds (`sd close` / `sd update`), write an ADR when a
  decision crystallized, `sd sync` + push.

## Phase 6 - Reflect (MANDATORY - never skip, every cycle)

1. **Cost review**: what took longer than it should, what needed
   retries, what was missing at decision time? EVERY finding that
   implies a behavior change MUST become an edit to THIS skill file
   (user directive 2026-08-27: a reflection that does not refine the
   workflow is ineffective). Domain knowledge goes to mulch; durable
   preferences to memory; process lessons HERE. No finding may stay
   unrecorded. Deployment on this repo is ALWAYS `just up` (cached
   loop: SPA, binary, image, CLI, compose) - `docker compose up
   --build` builds nothing here and `cargo build -p <name>` drifts with
   upstream package renames.
2. **Learnings -> mulch**: `ml record <domain> --type
   <convention|pattern|failure|decision> --description ...` (+ evidence
   flags), then `ml sync`. Real insights only - no ritual filler.
3. **New demands -> seeds**: feature requests, bug demand, and gaps
   observed on the way are filed autonomously with `sd create` -
   never parked in chat. SEARCH BEFORE FILING (user correction
   2026-09-07): run `sd search` with the finding's key terms (failure
   mode, tool name, script path, error string) before every `sd
   create` - most autonomous-line failures already have a seed
   (rate-limit windows, watchdog, journal hook, mise trust all did).
   When covered, extend THAT seed with the fresh run evidence
   instead of filing a duplicate. OWNERSHIP ON FILING (ADR-0018):
   seeds land unassigned by default; assign `@fabro` immediately ONLY
   for clearly-line work (it is a proposal, vetoable by reassignment);
   design forks, grill topics, and user-decisions get `needs-user`.
   Every unassigned seed appears in the report's ASSIGNMENT PENDING
   section (step 5).
4. **Open forks ahead**: note uncertainties and upcoming pivotal
   decisions for the next grill-with-docs; the user makes weichenstellende
   calls.
5. **Cycle report**: compact summary - outcome, verification evidence,
   seeds filed/closed, skill changes made. Every seed listed with a
   ONE-LINE DESCRIPTION, never a bare id (user directive 2026-08-27).
   ALWAYS include an ASSIGNMENT PENDING section: every seed filed since
   the last report that is still unassigned (revisor seeds land
   unassigned per ADR-0018 D2), each with a one-line @fabro
   recommendation - the cycle report is the standing joint forum where
   user + agent decide execution ownership (ADR-0018 D3). BETWEEN
   cycles, the persistent line-watch heartbeat carries the same
   ceremony: new revisor seeds of the approved classes (per the
   2026-09-07 curation: workflow/revision/engine/ci/web/docs work) get
   @fabro as a proposal, design forks and user decisions get
   needs-user and stay unassigned + flagged; every assignment is
   surfaced in the heartbeat report. Categorize EVERY revisor seed -
   none stays silently unassigned (user directive 2026-09-07). Then
   end the turn; the user starts the next cycle with /iterate.

## Standing rules

- Ownership boundary (ADR-0018, user decisions 2026-09-07): the
  autonomous line works ONLY on seeds assigned to `fabro`
  (`sd ready --assignee fabro` is the planner's sole candidate source,
  fail-closed). The revisor files seeds UNASSIGNED - reviewing is not
  owning. The agent's `@fabro` assignment is a proposal; user + agent
  decide execution ownership jointly (cycle report = standing forum,
  ASSIGNMENT PENDING section). User-owned seeds (grill sessions,
  design forks like fabro-3b1b, upstream-posture) stay unassigned.
  Emergencies bypass the picker entirely: line down -> agent repairs
  directly with the user's knowledge, seed filed retroactively.
- Line-health watchpoints during monitoring: verify on_overlap=skip
  survived any automation change (UI/replace wipes it, fabro-fb16
  class); a parallel conductor pass means the policy was lost; a
  pass parked on Tracker-empty with a non-empty backlog means the
  assignment queue ran dry - surface it in the report, never
  bulk-assign behind the user's back.
- Tool-agnostic engine (ADR-0017, user decision 2026-09-07): fabro
  engine components (sandbox providers, workflow engine, server, CLI)
  never reference project-scope tooling by name or behavior (mise,
  asdf, direnv, nvm, ...). Project tooling bootstrap lives in project
  artifacts: .fabro/Dockerfile.toolchain, server environment env,
  workflow hooks/scripts. The engine's stable contract is the clone
  layout (/repos/<owner>/<repo>, /workspace/<repo> symlink). Any
  'bootstrap tool X in run containers' demand maps to project
  artifacts first, never to engine seams.
- Upstream posture: we offer nothing until upstream reacts to our open
  issues/PRs (fabro-f251 parked, no priority).
- The two worlds are temporary (fabro-a9bb): the envelope family feeds
  the retirement. The move timing is decided jointly - when close,
  propose a dedicated grilling session instead of drifting.
- Boundaries: mulch = expertise, seeds = actionable work, ADRs =
  decisions, this skill = process. Nothing stays in chat that belongs
  in one of them.
- Test-only helpers behind the test-support feature; strum for enum
  string/int conversions; AGENTS.md Rust import style - these
  fabro-specific rules sit ON TOP of rust-style-guide, never against it.
