# Improve review — run 01M12DCDACAM5DE5MVHPZ50M21

- workflow: interview
- branch integrated: denkhaus-lab
- status: succeeded (completed), 4.3 min, $0.013752
- generated: 2026-08-27 22:15+0200 by `fabro ask` with `scripts/prompts/improve.md`

---

# Improve-review — run `01M12DCDACAM5DE5MVHPZ50M21` ("Interview")

**Run facts (from run events):** 7 nodes, 5 human gates, 1 model call. Wall time 258s; 195s of that (76%) was human-gate wait (yes_no 14.5s, confirmation 5.5s, multiple_choice 28.9s, multi_select 26.8s, freeform 119.3s); the only inference was `summarize` at 26.6s / $0.0138. Zero retries, zero failures, natural exit. Cost is not a lever in this workflow — human time and latency are.

## 1. Make the edges actually branch — or delete the gates that can't
**Evidence (from workspace file `.fabro/workflows/interview/workflow.fabro`, lines 42–56):** every labeled edge pair collapses to one target — `yes_no ��� confirmation` for both Y and N, `confirmation → multiple_choice` for both options, all four `multiple_choice` edges → `multi_select`, all four `multi_select` edges → `freeform`. The run confirms it: all five gate answers were recorded, but none changed routing (`edge.selected` events, seq 33/45/57/69/81).
**Change:** in `workflow.fabro`, give distinct targets per label — e.g. `confirmation [N] → summarize` (its label promises "Stop after this, but still summarize" but actually continues), and fan `multiple_choice [G]/[R]/[T]/[Q]` out to four per-theme question nodes instead of one shared `multi_select`. If a gate genuinely has no branch semantics (like `yes_no` here), delete it.
**Effect:** the graph becomes "progressive" as its goal claims; the summary gets real per-theme content instead of only preferences *about* the summary (the model itself flagged "thin content under the chosen frame"); removes ~20s of pointless forced round-trips. Side effect: the misleading journal entry at seq 69 (engine picked `[S] Success criteria` as the "selected" edge for a D,B,S,N answer, because all edges share a target) disappears.

## 2. Support free text alongside choices — this was the user's actual question
**Evidence (from run events, seq 78):** the user spent 119s — 61% of all human wait — typing into `freeform`: *"gibt es die Möglichkeit auch multiple choice fragen mit einer freitext antwort zu kombinieren?"* And the run itself answers it: every choice gate's `interview.started` event shows `allow_freeform: false` (seq 28, 40, 52, 64). The workflow had no node that could respond, so the question died in the summary as an "open question."
**Change:** allow `allow_freeform=true` on `multiple_choice`/`multi_select` nodes in `workflow.fabro` (the `freeform=true` edge attr already exists as precedent), or pair each choice node with an optional follow-up freeform node gated on an `[O] Other` edge.
**Effect:** nuance is captured at the point of choice; the terminal 2-minute freeform gate shrinks or disappears; the user's most engaged moment gets an answer path.

## 3. Clean up the summarize prompt context (duplication + ordering)
**Evidence (from run events, seq 88 — the stage prompt):** the German freeform text appears **three times** (`human.gate.freeform.answer`, `human.gate.label`, `human.gate.text` — the latter two are global single-slot keys overwritten by each gate), and the context table is alphabetical, not interview order, so the model must reconstruct chronology itself (it did, correctly, but that's luck, not design).
**Change:** stop emitting/persisting the global `human.gate.label`/`human.gate.selected`/`human.gate.text` keys (engine context handling), and render gate Q/A pairs in node order in the prompt — or drop the instruction "Use the `human.gate.<node>...` keys" and pass an ordered transcript directly.
**Effect:** smaller unambiguous prompt, zero risk of misattributed order, slightly fewer input tokens.

## 4. Lower reasoning effort for `summarize`
**Evidence (from run events, seq 89):** 1,891 reasoning tokens vs 660 output tokens for what is essentially transcription + light synthesis; 26.6s inference. Spec shows `controls.reasoning_effort: null`.
**Change:** set a low reasoning effort / high speed control for prompt-handler stages like `summarize`.
**Effect:** likely cuts the 26.6s summarize stage to single-digit seconds; cost impact negligible ($0.014 total).

## 5. Batch or skip checkpoints on zero-diff gates
**Evidence (from run events):** 6 checkpoints, every `diff_summary` = 0 files changed, metadata snapshots of 1.9–5.0s each plus a push, producing 6 empty run-branch commits (e858de4…640da65). Measured gate-to-gate stall: ~5s of checkpoint+push serialization after *every* answer (e.g. yes_no answered 20:08:29.26 → next question 20:08:34.31) — ~21s, 8% of wall time, all while the human is waiting for the next question.
**Change:** for interview-only graphs, checkpoint context on gate completion but skip the git commit/push when the diff is empty (or push once at terminal state).
**Effect:** next question appears ~5s sooner after each answer; run branch stops accumulating empty commits.

## 6. Make the interview delivery channel explicit in the spec
**Evidence (from run events):** spec shows `interviews: {provider: null, slack: null}`, yet all five questions were delivered and answered via Slack (actor `slack / tsu000nami`). The spec does not say where questions will go.
**Change:** populate `run.interviews` in the workflow/run spec.
**Effect:** reproducible, auditable delivery config instead of implicit server default.

## Error handling — honestly untested
Nothing failed in this run (0 retries across all nodes, `exit_kind: natural`), so there is no failure evidence to ground recommendations on. The one adjacent observation: the `freeform` gate accepted a question-as-context verbatim with no validation or clarification loop — if that matters, add a conditional re-ask edge rather than a generic retry policy.
