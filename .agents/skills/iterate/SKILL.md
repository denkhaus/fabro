# /iterate (fabro only)

One development cycle, end to end. The user starts a cycle by invoking
/iterate; the agent drives everything else. Chat replies in German,
every written artifact in English (ADR-0033) — in every repo of the
line (fabro, fabro-test, fabro-tofu): docs, ADRs, code comments,
workflow prompts, work-item notes, commit messages, PR bodies.

Scope: LOCAL session instrument for the human-side agent. Fabro
workflows never load or reference this skill; skills a run's agent
stages need are vendored into `.fabro/skills/<name>/`.

## How this skill is organized

The loop is seven phases, in order, every cycle. This file carries each
phase's binding rules and completion criterion. Mechanics, commands,
traps, and incident stories live in `reference/` — load the named file
WHEN its phase begins, never speculatively:

| File | Load at | Holds |
|---|---|---|
| `reference/orient.md` | Phase 0 | one-world invariants, sibling/roster protocol, PR sweep, ledger + review-gate arithmetic, friction score, line-state incidents |
| `reference/select.md` | Phase 1 | sprint composition, family order, the grilling format, lane tranches, vetting, local-first rules |
| `reference/build.md` | Phase 2 | design-first filing, mechanical gates, pin-power proofs, edit traps, Rust gates, vendoring, bench directives, verification recipes |
| `reference/review.md` | Phase 3–4 | two-axis review, language gate, presence suites, prompt fact-checks, battery sweep, salvage, environment-repair classes, the architecture gate |
| `reference/integrate.md` | Phase 5 | commit ceremony, tracker-write traps, push policy, deploy windows, tofu, toolchain pins, fork releases, bench landing |
| `reference/incidents.md` | on demand | the evidence index: every incident id, date, and one-line story, mapped to the rule it owns |

Routing (where knowledge lives, by phase need):

- `mulch prime <domain>` — incident expertise. Domains you will need:
  `rust` (edit mechanics), `testing` (verification discipline), `git`
  (push/PR discipline), `tooling` (tracker hygiene, bench landing),
  `nu` (scripting pitfalls), `engine` (over limit — prefer specific
  domains).
- Work items — the `fabro work` CLI; vocabulary and commands in
  `docs/agents/issue-tracker.md` (the word "seed" is retired).
- ADRs in `docs/adr/` — decisions this skill leans on: 0008 (graph+
  prompts+scripts as one unit), 0009 (stage-envelope family order),
  0012 (dogfooding), 0015 (serialization, one line one executor), 0017
  (tool-agnostic engine), 0018 (ownership boundary), 0019 (capability
  gate), 0021 (fork_line_recovery), 0022 (judgment pre-screens), 0025
  (fork release naming), 0027 (frozen deps), 0028 (complexity brake),
  0032 (central tracker), 0034 (vendored forks), 0036 (handoffs are
  transport), 0038 (observations are platform records).
- `docs/internal/*-strategy.md` + AGENTS.md — read the matching
  strategy doc BEFORE touching its area (logging, events, testing,
  secrets, migrations, errors, React effects).
- `.agents/skills/rust-style-guide` — THE binding Rust policy,
  identical for upstream and fork code.

## Phase 0 — Orient (always, cheap)

Completion criterion: you know the sibling situation, the line state,
the pending-gate state, the interrupted-cycle state (if any), and the
friction verdict — before anything is selected.

1. One world: everything works on the LINE branch `denkhaus` in the
   main checkout, plain git (`git pull --ff-only origin denkhaus`
   BEFORE reading tracker state). Runs execute on the PRODUCTION
   server — every line query carries `--server https://mirtuell.net`;
   the local server (127.0.0.1:32276) is for TESTS only. git worktrees
   never. Never checkout/reset while a background cargo runs.
2. Agent roster FIRST: before anything else, establish whether another
   top-level session shares this checkout. A shared checkout shares the
   uncommitted pool, the tracker file and the ledger; pick a disjoint
   surface, claim it, never commit a sibling's files, NEVER `git stash`
   (the stash stack is global). Protocols: `reference/orient.md`.
3. Open-PR sweep: `gh pr list --repo denkhaus/fabro --state open` +
   checks per PR — EVERY gh call carries `--repo denkhaus/fabro`
   explicitly (the default repo is a different one with colliding PR
   numbers). A red dogfood-gate run PR blocks auto-merge and wedges the
   push gate: repair BEFORE selecting. Mechanics: `reference/orient.md`.
4. Line state: `fabro ps` (mirtuell). A running pass plus empty queue
   is fine; a parked pass with a non-empty queue, a parallel pass, or a
   lost `on_overlap:skip` is an incident. While the workflow works the
   tracker, this session claims nothing — one line, one executor.
5. Interrupted cycle: reconstruct BEFORE selecting — `git status` plus
   in-progress work items name the mid-flight work; continue it.
6. Sprint ledger: read `iterate-state.json` at the repo ROOT. If
   `sprints_reflected < sprints_completed`, run the pending reflection
   (Phase 6) FIRST. Compute which review gates are DUE from the three
   markers (`%2` code-review, `%3` architecture, `%5` rust-security) —
   the full arithmetic and marker semantics: `reference/orient.md`.
7. Rootprint: production logs are queried through the `rootprint` CLI
   (skill at `~/.prime/agent/skills/rootprint/SKILL.md`); never read
   `docker logs rootprint-vector`; INFO is not ingested. For every run
   under evaluation, correlate its rootprint window with its journal
   painpoints into ONE work item.
8. Friction score: `nu .fabro/scripts/friction-score.nu` once per
   cycle. Verdicts gate Phase 4: `normal` (<0.30), `grind` (0.30–0.60),
   `architecture-due` (>=0.60).

## Phase 1 — Select

Completion criterion: a sprint of 2–3 hand-vetted items is composed
(and each item's design fork is grilled or already grill-recorded), or
the interrupted cycle's continuation is named.

- Pick the highest-value open items respecting family order (ADR-0009:
  900e -> 47b5 -> e47c -> e804 -> ba96), blockers, and the user's
  focus.
- Sprint composition: 2–3 items, selected together; a large item shares
  the sprint with at most one small companion; never a one-item sprint.
  The review cost is per sprint, batches amortize it.
- EVERY design/mechanism/standing-rule choice is grilled BEFORE
  implementation — run the ACTUAL grill-with-docs procedure, never an
  ad-hoc presentation. Rounds go through the `question` tool, options
  carry descriptions + the Jev pre-screen distribution, recommended
  option first, noul dimensions are their own question. Public-contract
  NAMES (graph attributes, API fields, tool names, CLI flags, settings
  keys) are pivotal: surface naming for user decision before the first
  commit. The full format: `reference/select.md`.
- Local-first: invoking /iterate means the session works LOCALLY; only
  loop-asset work (`.fabro/workflows/**`, `.fabro/scripts/**`,
  `scripts/**`, justfile, fabro-test rigs) is lane material. Until the
  re-armed line is PROVEN reliable, propose NO lane moves and NO line
  assignments — the cycle report recommends LOCAL work instead.
- VET BEFORE WORKING: every item considered gets a relevance test
  against the CURRENT codebase first (does the cited path/mechanism
  still exist? overtaken by landed work?). Lane admission is vetted
  one item at a time, never bulk. The rules and their reasons:
  `reference/select.md`.

## Phase 2 — Build (local-first)

Completion criterion: every sprint item is implemented under the
mechanical gates of its surface, verified with real commands, and
closed with named evidence — or parked with a filed continuation.

- File the design into the tracker BEFORE any code exists — design
  decisions live in the tracker, never only in chat or the diff.
- Claim (`work update --status in_progress`), implement, verify, close
  with evidence. Per-surface gates and the full discipline:
  `reference/build.md` — load it before the first edit cell. The
  non-negotiables:
  - Rust work: rust-style-guide SKILL.md AND the guideline pages
    covering the diff in the SAME turn, before the first edit; name the
    pages in the cycle report.
  - Pins are PROVEN, not assumed: after writing a regression pin,
    restore the pre-fix implementation and confirm the pin goes RED,
    then restore. A pin that cannot go red is blind.
  - Run the touched tests IMMEDIATELY after each refactor edit, not at
    the end of the batch.
  - Direct-fix verification (before any push): lint first, then
    dry-runs — positive AND negative, each its own shell call, exit
    codes read from the process, never behind a pipe.
  - Every `.fabro/workflows/**` change lands WITH a bench surface in
    fabro-test that exercises it (a dedicated probe, or a named
    existing row that provably drives the changed asset); a change
    without one files the follow-up in the same session.
  - Bench philosophy: targeted single-aspect probes that compose; never
    rerun the full bench for one red facet; after every test round,
    cross-check the platform logs (rootprint) for the window's
    WARN/ERROR against the green table.
  - A line re-arm is a RELIABILITY CHAIN, never a default: blockers
    landed, one manual supervised lap per lane (each log-cross-checked),
    then the USER decides.

## Phase 3 — Review

Completion criterion: the batch's whole accumulated diff since the
previous sprint's tip passed both review axes (findings folded or
filed), the language gate is clean, and every battery named in
`scripts/qualitygate.nu` ran green.

- ONE review per sprint over the batch's diff (frozen to a patch file
  first): the code-review skill, two axes (standards = rust-style-guide
  + strategy docs; spec = the tracker's design bodies). Fixpoint
  derivation and the execution shape: `reference/review.md`.
- Language gate (ADR-0033): every reviewed artifact is English. A
  German paragraph or mixed-language file in the diff is a FINDING —
  fix in the same cycle, translating the whole file at once.
- Landed prompt diffs get a FACT-CHECK against repo reality, and prompt
  hygiene: no seed-id literals, run ids, PR numbers, commit shas, dated
  cost narratives, machine-specific paths, or decision-record
  references inside prompts — provenance lives in graph comments and
  the tracker. The mechanical net is the prompt-lint evidence ban.
- Fork-feature regression check: run the fork-only presence suites of
  every crate you touched; every NEW fork feature carries a presence
  pin (fork-only test file + TOUCHPOINTS row). Red fork-only test =
  regressed fork feature: fix or revert, never relax the test.
- Loop-asset battery sweep before declaring a gate healthy: run EVERY
  battery named in `scripts/qualitygate.nu`, read each exit code.
- Salvage: a failed (or green-lie succeeded) run with noteworthy stage
  work is SALVAGED — dump, apply, verify, land citing the run, or file
  a salvage-pointer item. Never let stranded work vanish silently.
- Environment artifacts vs regressions: compile errors on untouched
  deps, expect-panics across sandbox suites, and default-profile
  timeouts under dev-host conditions are repair-the-environment-first
  classes — the discrimination recipes: `reference/review.md`.

## Phase 4 — Deepen (conditional)

Deepen only when review surfaced structural smells OR the Phase 0
friction verdict is `architecture-due`; `grind` deepens only on smells;
`normal` never deepens on score alone. The architecture gate fires the
LOCAL improve-codebase-architecture agent (the architect WORKFLOW stays
deactivated on the line); its top recommendations become work items,
worked LOCALLY. Architect scope is binding: findings restructure ONLY
the fork's own surface; `lib/**`/`apps/**` is observable, never
restructurable. The full gate procedure: `reference/review.md`.

## Phase 5 — Integrate

Completion criterion: code committed (named files, message through a
file, read back), pushed behind an OPEN gate, the tree clean, and any
deploy window completed through its proof ceremony with its pins
committed.

- Commit BEFORE tracker mutations land. Stage NAMED files only (never
  `git add -A`); the message travels through a file (`git commit -F`),
  written and verified THIS session; read it back before pushing.
- Push policy: during the cycle, branch updates and read-only
  integration stay allowed; the PUSH direction is gated to one
  mechanical decision at the END — `nu .fabro/scripts/push-gate.nu`
  (exit 0 = open), UNPIPED, check and push share one cell. All the
  shell-RC traps: `reference/integrate.md`.
- Deploy windows: bench gate first (full workbench green or no window),
  pause the line, deploy noninteractive, then ONE live proof of the
  shipped behavior — never report success from version strings alone.
  The complete recipes (tofu, pins, fast path): `reference/integrate.md`.
- Secret-gated steps (tofu/gopass) are probed with a BLOCKING
  `gopass show` WITHOUT a timeout wrapper BEFORE the step; ask the user
  to warm the store EARLY, not at deploy time.
- Fork releases follow ADR-0025 naming `<frozen-base>-fork.N`; the
  manual bump recipe and the toolchain-repin ordering rules:
  `reference/integrate.md`.

## Phase 6 — Reflect (mandatory, every sprint close)

Completion criterion: the Zusammenfassung + Ausblick is delivered, every
cost finding is recorded in its proper store, the ledger carries the
sprint record (counted per the rule), and the heartbeat matches
reality.

1. END-OF-SPRINT CEREMONY: every sprint close ends with a Zusammenfassung
   + Ausblick in the chat, WAIT-WHAT style — context first (which
   sprint, which items, where landed), then what was done with
   verification evidence, then the AUSBLICK: the next sprint's items
   and WHERE the user is needed (decisions, gates, credentials,
   reviews). Simplified short sentences, the repo's ubiquitous
   language; chat stays German. The next sprint starts IMMEDIATELY
   unless a user gate blocks it.
2. Cost review: what took longer than it should, what needed retries,
   what was missing at decision time. EVERY finding lands in its proper
   store — behavior change => edit THIS skill (the ABSTRACT rule only;
   session state and incident detail go to the ledger note, the
   tracker, or mulch via `mulch record <domain> '<content>' --type
   <failure|convention|pattern|decision|reference|guide>` +
   `mulch sync`); durable preferences to memory. No finding stays
   unrecorded.
3. New demands become work items, searched-first for duplicates.
   Ownership per ADR-0018: unassigned by default; design forks and
   user-decisions get `needs-user`.
4. Cycle report: outcome, verification evidence (commands run), items
   filed/closed — each with a ONE-LINE description, never a bare id —
   and ALWAYS an ASSIGNMENT PENDING section: every unassigned item with
   a one-line recommendation.
5. Sprint ledger update: a sprint counts ONCE when it closes at least
   one item this session worked LOCALLY with a substantive diff; line
   closures never count; docs-only and verify-only closures never
   count. Notes live in the `sprints` array, ONE record per sprint,
   several notes possible. Rotation: `just rotate-notes` when sprints
   > 10. The counting history and rotation mechanics:
   `reference/orient.md`.
6. Heartbeat cadence tracks reality: 20m while anything autonomous
   runs (pass, build, deploy chain); PAUSED (not 60m) when everything
   is user-gated or nothing runs. Create or resume only when autonomous
   work is actually in flight; refresh the instruction at every phase
   change — a heartbeat still carrying a completed critical path is
   noise that erodes trust. Heartbeats are session-scoped: recreate
   from this spec when missing; if the session host rejects heartbeats
   entirely, ask the user to set the visible /heartbeat with this
   ceremony — never run silently unwatched. The user's pause is
   authoritative and never overridden.
7. Line-watch: pull, evaluate journals/reviews (premise-checked
   against the tree), dispatch per ADR-0018 with dispatch-dedupe, push
   through the gate, rootprint correlation for every evaluated run,
   report compactly in German. SALVAGE SWEEP is mechanical first:
   `nu .fabro/scripts/salvage-sweep.nu` and route its verdicts — the
   SESSION owns this call (it holds the server login; a run sandbox has
   none).

## Standing rules

- COMPLEXITY BRAKE (ADR-0028): a SECOND patch on the same mechanism
  within days is a smell — the mandatory response is simplify-or-REVERT
  (citing the owning ADR), never a third patch. A patch contradicting a
  standing decision stops for an explicit supersession, never a silent
  additive layer.
- WRITTEN LANGUAGE IS ENGLISH (ADR-0033): chat German, artifacts
  English, every repo. A German file never licenses German additions;
  translate the whole file in the same change. Only test data that
  deliberately probes non-English output stays non-English, named as
  such.
- NO LANGUAGE SPRAWL (user directive 2026-10-03): every script invoked
  by workflows, justfile, or gate/tooling entry points is a nu script.
  POSIX sh survives only where the failure mode guards a sandbox
  WITHOUT nu. Architecture scans and sprint plans check the language
  axis BEFORE proposing splits.
- Style-guide pair invariant (fabro-6538): `.fabro/skills/rust-style-guide/`
  is canonical, `.agents/skills/rust-style-guide/` its synced mirror —
  after any edit run `just sync-style-guide`. Loop runs never write
  `.agents/**`.
- VCS is PLAIN GIT (user decision 2026-10-09): named-file staging,
  commit -F, ff-only pulls, pushes only behind the gate; never merge
  origin/main (upstream intake frozen, ADR-0024).
- Wait budgets derive from observed service latencies, not guesses: a
  bounded wait must outlive the slowest LEGITIMATE stage of what it
  waits for; a fallback bypassing a verification path re-checks the
  gated path one last time and cleans up its ghost.
- Judgment pre-screens (ADR-0022, session-side integration, user
  directive 2026-10-16: use it everywhere or never get evaluation
  data): advisory, fail-open, via `.fabro/scripts/judgment.nu`
  (criteria is an option-id -> description RECORD, never a bare array);
  every call logs to `~/.local/state/fabro-judgments/<date>.jsonl` —
  the log IS the evaluation corpus, so a phase that CAN pre-screen and
  does not is a finding. Application points: (a) every grill fires a
  pre-screen BEFORE the user grill; (b) cycle review — verdict per
  evaluated run and per-file residue adjudication; (c) line-watch
  journal routing — painpoint severity as noul before dispatching;
  (d) salvage verdicts — noteworthy-or-not against the sweep's call;
  (e) PR triage when a merger lane exists (parked in fabro-c14e's
  demand). Thresholds only after the corpus report; until then the
  answers advise, the documented user decision decides. Format detail:
  `reference/select.md`.
- Security-hole closures are DIRECT agent work, never line work: the
  fixer must not stand in the trust circle being closed.
- Capability gate (ADR-0019): agent sandboxes get least privilege; all
  forge writes are engine-mediated; any item changing a tool,
  credential, or permission in an agent-reachable surface is needs-user
  until approved; a merged capability change without a recorded user
  decision is reverted, not ratified.
- Loop-asset fix surfaces (`scripts/**`, `.fabro/**`, `justfile`) are
  unlandable by the develop line — the fs envelope kills the write.
  DIRECT-FIX PROPOSAL DUTY: at filing/triage time, every such item (or
  trivially mechanical one-file fix with evidence in hand) is PROPOSED
  to the user as a direct fix in the same turn — never silently queued.
- Ownership (ADR-0018): the line works only fail-closed-picked items;
  the run-end observation filing files items UNASSIGNED. Emergencies
  bypass the picker: line down -> agent repairs directly with the
  user's knowledge, item filed retroactively.
- Line-health watchpoints: `on_overlap=skip` must survive every
  automation change; after each pass, confirm the child PR merged into
  `denkhaus` and the work commits appear in origin/denkhaus.
- Tool-agnostic engine (ADR-0017): engine components never reference
  project-scope tooling by name.
- Upstream posture (ADR-0024 + ADR-0027): the platform base is FROZEN
  at 0.362.0-nightly and the ENGINE DEPENDENCIES are frozen — a dep
  bump is a deliberate decision with a recorded reason and its pin edit
  in the same change. Keep the `.fabro` layer portable; still
  selectively offer general, non-strategic fixes upstream; strategic
  assets stay fork-private. Re-open condition: the engine structurally
  blocks a change we cannot make in the fork.
- Dependency finish: a session that lands work in an engine dep
  FINISHES it before it ends. Since ADR-0034 the forks are IN-TREE
  (`vendor/petri`, `vendor/pebble`): the vendored edit, the consuming
  change, and the fork export (`just vendor export <name>`) are ONE
  change; session-close check `just vendor status` clean +
  `just vendor-pins` green. Dependency work never ends silently
  unfinished — an impossible export leaves a handoff item naming fork,
  branch and local state EXPLICITLY.
- Fork-feature presence pinning: every durable fork feature lives in a
  fork-only source file wired through minimal one-line seams, with a
  presence test in a fork-only test file plus a TOUCHPOINTS row.
- Session/line boundary (fabro-cadd lesson): session bookkeeping
  (ledger, reflection, gate cadence) is iterate-skill-LOCAL at the repo
  root; anything proposing to port it into line assets is
  needs-user/grill-first. The architect agent is local, permanently.
- Boundaries: mulch = expertise, work items = actionable work, ADRs =
  decisions, this skill = process. Nothing stays in chat that belongs
  in one of them. Fabro-specific Rust rules (test-support feature,
  strum, import style, shell_quote) sit ON TOP of rust-style-guide.
- HANDOFFS ARE TRANSPORT ARTIFACTS, NOT HISTORY (ADR-0036): the moment
  a handoff's demands are consumed (landed or filed), the handoff is
  deleted — a markdown file `git rm`ed, an item-handoff CLOSED with
  evidence, references cleaned in the same change. A consumed handoff
  left in the tree is a finding.
