# Revision — run 01M2VMMHM71C15BVASZSYXWFN8

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2VMMHM71C15BVASZSYXWFN8.md
- seeds filed: none — no credit this pass (ADR-0022)
- balance: 0 non-exempt seeds filed / 0 — no credit this pass
- basis: run 01M2VMMHM71C15BVASZSYXWFN8, workflow version 2edfd65e52fc7543478409763d768a38506d2a554f690d1f98efed0a5be796b0, commit 40eaa32b6776bffdf92630380fe5e6f2ebd39ca7
- revised_at_commit: 40eaa32b6776bffdf92630380fe5e6f2ebd39ca7 (ADR-0015: engine drift signal for later judgement)

## Findings

Both findings survived dedupe review (distinct from open fabro-b8ed, closed
fabro-f7cf, and the re-file family fabro-534e/fabro-7aac/fabro-22fa) but this
pass earned no filing credit (zero same-pass stale/superseded closes), so under
ADR-0022 they are journalled as overflow for the next pass to re-file after
re-running its own dedupe.

### 1. Fix the dead tester-gate criterion in the Verification-only brief template

- filed: none — overflow to journal (no credit this pass)
- concrete change: in `.fabro/workflows/develop/prompts/planner.md`, the
  Verification-only JSON brief template's gate bullet says "gate green via the
  deterministic tester step", but the Verification-only edge
  (planner -> evidence, fabro-9d26) skips the tester by design, making the
  criterion structurally unsatisfiable; in this run the reviewer improvised its
  own nextest scope (`cargo nextest run -p fabro-types -p fabro-workflow context_keys preamble`)
  to compensate. Replace the gate bullet with "targeted tests green via named
  nextest filter (timeout_ms >= 600000)".
- expected effect: verification-only briefs carry only checkable criteria and
  reviewer test scope stops being improvised.

### 2. Closeout: honor seed-body Close-when arms before sd close

- filed: none — overflow to journal (no credit this pass)
- concrete change: `.fabro/workflows/develop/scripts/closeout.nu` closes the
  claimed seed without reading close-conditions written in the seed body:
  fabro-6a78's body says close when upstream PR #786 merges AND denkhaus
  carries the merge, this run's reviewer journal warned the deterministic
  closeout bypasses that external arm "by design", and the run had to
  hand-roll the holding state (PR #254 moved the seed to in_progress pending
  upstream) because closeout has no externally-gated branch. Change: before
  `sd close`, grep the seed body for Close-when arms; unmet external
  conditions keep the seed open or re-file a low-priority tracking seed (or
  record them in the close reason).
- expected effect: externally-gated obligations stay tracker-visible instead
  of relying on manual in_progress holds. Extends the re-file family
  (fabro-534e EXEMPTION arms, fabro-7aac implementation-summary deferrals,
  fabro-22fa reviewer-journal findings) with the seed-body arm none of them
  covers.
