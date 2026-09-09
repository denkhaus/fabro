---
name: integrate
description: Review and integrate a batch of incoming line commits (run PRs merged on another machine). Pull, triage EVERY commit, review along six fixed axes (features, gaps, misconceptions, policy/capability, code quality, docs completeness), reconcile the seed tracker (closure verification, duplicate elimination, stale-basis detection), verify tests at HEAD, file follow-up seeds, and push only inside safe run-PR windows. Use when the user asks to pull and review the autonomous line's landed work, audit incoming run PRs, or after a multi-run session on another machine. Complements the iterate skill (cycle work) - integrate owns the INCOMING side.
---

# /integrate (fabro only)

One incoming-batch review, end to end. Chat replies in German, all
written artifacts in English. The autonomous line implements; this
skill reviews, reconciles, and integrates. Never implement seeds here
(except standing agent-exceptions recorded in iterate).

## Phase 0 - Preconditions (order matters)

1. `git fetch origin --prune` + `git pull --ff-only` BEFORE reading any
   `sd` state - the tracker view is branch-local and goes stale the
   moment another machine runs the line (2026-09-08: closed seeds read
   as open from a pre-pull dump).
2. Diverged (local commits + incoming)? Merge is fine - but EVERY merge
   touching `.seeds/issues.jsonl` or `.mulch/**` gets the JSONL
   discipline below BEFORE anything else continues.
3. Window snapshot for later pushes: open run PRs, `git ls-remote
   --heads origin 'refs/heads/fabro/run/*'` newest branch, line state.
   A run branch head younger than the newest merged PR = run in flight.

**JSONL discipline (PR #81 + 2026-09-09 lesson):** git can merge
`.seeds/.mulch` "cleanly" and STILL duplicate lines. After every merge:
count duplicate ids; resolve by keeping the later `updatedAt` line; NEVER
drop an `assignee` state (assignments are user-ratified decisions); a
clean merge is not a correct merge. Verify zero duplicate ids, then
commit the dedupe explicitly.

## Phase 1 - Commit triage

Categorize every incoming commit before deep-diving:

- **Implementation PRs** (run squash-merges, `(#N)`, stage markers in
  the message) - full six-axis review, biggest diffstat first.
- **Revisor passes** (file/dedupe seeds, run reviews) - review the SEEDS
  they filed, not just the counts.
- **Process/skill edits** (iterate skill, AGENTS.md) - check against
  this repo's reality, not the author machine's.
- **seeds: sync commits** - mechanical, but they carry closure events:
  every seed that flips to closed here gets Phase 3 verification.

Build the ledger first: commits x seeds-closed x seeds-created, so
nothing is reviewed twice and nothing skips review.

## Phase 2 - Six review axes for every implementation commit

Run all six on every implementation PR. Findings land in the report
under the same headings; every finding names its evidence (seq, run id,
commit, line).

### Axis 1 - Features: what actually landed

- Open the SEED spec next to the diff. Every user-approved design point
  (grill Q1..Qn, acceptance bullets) must be LITERALLY visible in the
  artifact - an edge, an attribute, a test, a prompt line. Approved-but-
  invisible forks are the most expensive drift class (iterate: fabro-7461).
- Distinguish "implemented per spec" / "partial with acknowledged gap"
  / "implemented something else".

### Axis 2 - Gaps: what is missing or fragile

- Known-limitation comments in the diff (in-memory state, reset windows,
  acknowledged TODOs) - list them, they are future seeds.
- Interaction risks with OTHER landed work (2026-09-09: the PR
  staleness supervisor's 409-as-strike counting can prematurely retire
  PRs GitHub answers 409 for while an update is merely queued).
- Scope beyond the recorded approval: a mechanism may generalize wider
  than the user decision that authorized it (fabro-c419: approval was
  one read-only binding, the engine shipped the general opt-in). Always
  surface the delta; the user ratifies, never the diff.

### Axis 3 - Misconceptions: wrong or stale premises

- Decode cited run ids (ULID timestamps) and verify cited commits/files
  exist and say what the seed claims.
- Premises about RUNTIME state are invisible from the tree: verify
  against the live source - repo settings, PR objects, server state
  (2026-09-09: fabro-6a5a disabled auto-merge on the premise "the
  enableAutoMerge call deterministically fails", evidence from 09-02;
  live check showed allow_auto_merge=true and the call working - #102
  and #107 itself carried active auto-merge. The line implemented a
  config change on a stale premise and a one-word user decision
  reverted it).
- Machine-specific facts in repo-shared files are defects (container
  names, host paths, "runs execute on X" claims - true only on the
  author machine).
- Outdated framing check: grep for retired vocabulary (two-worlds,
  retired branch names) - zero leftovers is the bar.

### Axis 4 - Policy, security, capability boundaries

- ADR-0019/0020 apply to EVERY diff: tool, credential, allowlist,
  permission, or fs_hide changes in agent-reachable surfaces (Docker
  files, environment env, workflow graphs with fabro_tools/fs_hide,
  hooks) need a recorded user decision. A merged capability change
  without one gets reverted, not ratified.
- Graph-declared per-node tools (fabro_tools) are reviewer-gated power
  (ADR-0020): the declaration must be the minimal set the stage's job
  requires; an unjustified escalation is a review miss.
- New code paths USING an engine-provided credential or bridge are a
  capability delta too (ADR-0019 item 7) - "the token already existed"
  is not an argument.
- Trust-boundary violations: fixers must not stand inside the circle
  they harden (security closures are agent work, never line work).

### Axis 5 - Code and logic quality

- Test classes present? Wire tests against mocked HTTP for forge
  integrations, round-trip tests for event/JSON contracts, regression
  tests reproducing the incident's exact state shape (fabro-91ff:
  waiting-on-child AND terminal-parent-live-child), handler-boundary
  tests for structured-output validation.
- Wiring complete? Supervisor spawns + shutdown aborts in serve.rs,
  run_state/event-contract updates, store query bounds.
- Then verify at HEAD yourself: `cargo nextest run -p <touched crates>`
  after the pull (2026-09-09: 3133 tests green across five touched
  crates). A green per-PR gate does not replace an at-HEAD batch check.
- Twin-mode trap: `--profile e2e` NO-OPs twin tests; run those in the
  default profile (iterate: fabro-47b5).

### Axis 6 - Docs completeness for new features

Every landed feature answers: does the source of truth know about it?
- OpenAPI spec (`docs/public/api-reference/fabro-api.yaml`) for new
  HTTP surfaces - spec first, then progenitor/types (AGENTS.md API loop).
- `AGENTS.md` architecture sections for new crates/components/commands.
- `PROJECT_FACTS` / develop prompts when engine behavior the line
  depends on changed (PR supervisor semantics, tool registration).
- ADR when a decision crystallized; strategy docs when the touched area
  has one (logging, events, testing, errors, migrations, secrets).
- Missing docs = gap finding, seed-worthy, not a chat note.

## Phase 3 - Tracker reconciliation

1. **Closure verification (fabro-9967/a0e3 lessons):** every seed that
   flipped to closed must show its demand in a merged diff. If not:
   grep `.fabro/journal/*.jsonl` and `.fabro/revisions/*.md` for the
   seed id - absorption/superseded closures hide their reason only
   there. A closure that is neither diff-visible nor documented
   anywhere is a tracker-hygiene defect: file it (reason must be
   recoverable in the seed record, not only in journals).
2. **Duplicate elimination:** before filing ANY new seed, `sd search`
   the finding's key terms (failure mode, tool name, error string,
   path). Most findings already have a seed; extend THAT one with fresh
   run evidence instead of filing a sibling.
3. **One problem class = one durable fix:** map findings to their
   lineage (duplicate-claim: fabro-22e4 -> 91ff -> 6a5a-race; stale-
   basis: 01b9 -> 4814 -> 6a5a; closure-opacity: a0e3/9967). N patches
   for one class means the class is not fixed - the durable fix is
   usually engine-side serialization or validation, not another prompt
   line. Say so in the report.
4. **Deprecated/stale seed elimination:** seeds whose Basis no longer
   resolves against the tree close as superseded (with reason + what
   changed); seeds whose demand the tree already implements close with
   the implementing commit as evidence. Every close follows the
   closure discipline: reason + evidence in the seed record BEFORE
   `sd close`.
5. **Same-problem-different-form check:** when two commits/seeds look
   unrelated but describe one underlying defect, name the shared class
   and unify the follow-up (do not file per-symptom seeds).

## Phase 4 - Follow-up derivation

- Every gap/misconception/doc miss becomes a seed (search first,
  Phase 3.2) with: self-contained description, acceptance criteria,
  Basis line (run id, workflow version, commit), ownership per
  ADR-0018 (line work -> @fabro proposal; design forks and user
  decisions -> needs-user + unassigned; capability/security -> agent
  or needs-user, NEVER fabro).
- Weichenstellende decisions do not get implemented - they get a grill
  session; the agreed design goes INTO the seed before the line sees it.
- Decisions that crystallized -> ADR. Expertise -> `ml record`. Process
  lessons -> edit THIS skill or iterate.

## Phase 5 - Push coordination (run-PR windows)

Pushes to the base branch are the only step that can damage in-flight
line work. Rules:

1. **Safe window = zero open run PRs AND zero in-flight runs** (a run
   branch whose head is younger than the newest merged PR, or any
   non-terminal develop/conductor run). The claim-to-PR window is the
   whole run duration: a run started before your push opens its PR on
   a stale base.
2. A push outside the window dirties run PRs: auto-merge stalls, the
   staleness supervisor update-branches them, and JSONL conflicts turn
   into 3-strike `stale_base` retirement of work that was converging.
   Until the 409-strike fix lands, treat every dirty run PR as urgent:
   update its branch manually (merge base into the run branch, JSONL
   union-resolve) BEFORE three 5-minute strikes pass.
3. Window sequence: let/merge pending run PRs (gate green) -> pull +
   JSONL discipline -> push -> verify origin state (the config you
   intended is what origin serves).
4. **Duplicate-run containment (claim races):** a run whose branch base
   predates a just-merged implementation re-claims the closed seed
   (2026-09-09: claimed 40 s before #107 merged). Detection: the
   in_progress flip on a run branch whose base lacks the closing
   commit. Containment: the duplicate PR gets closed unmerged on sight
   with a duplicate-run comment BEFORE its gate can go green - and the
   claim-race evidence extends the serialization seeds (engine-side
   fix, not another prompt guard).
5. Gap runs: a run started between a config change landing on base and
   the next pass runs under the OLD config snapshot (settings resolve
   at run creation - fabro-dc81 class). Expect one stale-config run
   after every config-flipping merge; check its output against the
   decision, do not let it silently re-land the old behavior.

## Phase 6 - Report (German, compact)

1. Gesamtbild: commit count, span, composition, at-HEAD verification
   results.
2. Per commit: achieved / gaps / misconceptions - evidence-named.
3. Aggregated gaps + misconceptions with severity.
4. Tracker reconciliation: closures verified, defects found, seeds
   eliminated.
5. Follow-ups filed: every seed one line (id + description), plus ADR/
   skill/mulch changes made.
6. ASSIGNMENT PENDING: every unassigned seed with a one-line ownership
   recommendation (user + agent decide jointly - the report IS the
   forum).
7. Push status: window assessment, what was pushed, what is deferred
   and why.

## Standing rules

- The report's factual premises get verified, not trusted: a reviewer
  (human or agent) asserting a runtime state gets the same live check
  as a seed premise (Axis 3).
- Nothing reviewed here stays chat-only: gaps -> seeds, decisions ->
  ADRs, expertise -> mulch, process -> skill edits.
- Engine-mediated reads only for forge state during review; operator
  interventions on running infrastructure are OUT of this skill's scope
  (they are one-off repairs, not process).
