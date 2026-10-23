# Improve review — run 01M5KJ2NWJP1TM6JZZKQ2KHNX5

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (90.7s, revisor pass — reason and cost in run detail)
- generated: 2026-10-23 07:16+0200 by revisor `fabro_ask`

---

All evidence below is from this run's event stream (stage transcripts, planner tool calls, conclusion timings/cost, journal payload) and the live tracker (dedupe searches named per item).

**What this run actually was:** a 90.7s scheduled loop run that never reached implementer — tracker_guard requeued stale-claimed `fabro-30a5`, preflight flagged it `duplicate`, and the planner spent 59.7s / $0.067 / 23.5K input tokens / 5 LLM turns / 5 shell calls closing one seed via the two-branch rule, then exited "Already landed". The run's only diff (2 files, +6/−1) is tracker bookkeeping — yet a full PR #482 was generated for it.

## Recommendations, by expected impact

**1. Give the no-change / verification-only route a deterministic close arm.**
- Change: `.fabro/workflows/develop/scripts/closeout.nu` (+ the loop/develop closeout prompt): when the run diff contains no loop-asset change and the reviewer approved a verification-only pass, close the seed with reason `verified: no changes (run <id>)` instead of skipping.
- Evidence (from run events): run `01M5KGBR2BAS8BKYKG06PEVHMZ` (PR #480) completed planner→evidence→reviewer→closeout all green yet left `fabro-30a5` open — `tracker_guard` had to requeue it (`"requeued":["fabro-30a5"]`), and this entire run existed to close it. The planner journaled this exact painpoint. Note the gap is structural: closed seed fabro-02c4's "demand visible in run diff" discipline can never be satisfied by a no-change run.
- Effect: eliminates one whole scheduled run (~90s, ~$0.07, plus a PR) per stranded no-change seed.
- Tracker: new item. Searches `closeout close seed no changes verification-only` and the full `closeout` list (13 rows) show no open seed covering the no-change route; fabro-02c4 (closed) created the rule but not the exception.

**2. Skip (or force-draft) PR creation when the run diff is tracker-only.**
- Change: run `pull_request` settings / loop automation guard: suppress PR when `run.diff` touches only `.seeds/**`.
- Evidence: PR #482 was requested at 05:01:49.5 and created at 05:02:57.2 — **67.7s of glm-4.7 PR-writing**, roughly the length of the entire workflow body, for a diff that is 100% tracker bookkeeping (seed requeue + close rows).
- Effect: ~1 minute and one review-noise PR + Slack ping saved per bookkeeping run.
- Tracker: new item. Searches `pull request tracker-only skip draft` and `PR no code changes skip` returned 0; open fabro-cf31 covers PR *title* derivation, not suppression.

**3. Adopt the top-N ready view (existing fabro-c3b4).**
- Change: implement **fabro-c3b4** (open) — top-N view in the planner instead of the full `seeds ready` firehose.
- Evidence: the planner's first shell call returned 65 rows / 9.4KB with `stdout_truncated: true`; it used only the top row. Conversation+memory consumed 19.9K of the 23.5K input tokens.
- Effect: ~9KB less tool output every planner pass, no truncation, faster candidate pick.

**4. Strip the static legend from per-run preflight output.**
- Change: `.fabro/workflows/develop/scripts/planner-preflight.nu`: remove the `legend` object from the emitted JSON; keep it once in the planner prompt/docs.
- Evidence: preflight stdout was 3,723B, of which ~2.4B is the static legend; it was paid ~3× per run — inlined into the planner preamble (`x.context_allow_keys="output.preflight"`), echoed in `command.output`, and carried into the final invocation context.
- Effect: ~2.4KB less preamble per planner visit — multiplied across revisits (planner `max_visits=20`).
- Tracker: new item. Searches `preflight legend output` and `preamble tokens preflight planner context` returned 0; fabro-c3b4 covers only the ready-list.

**5. Emit `closure_run` on preflight `duplicate` rows.**
- Change: `planner-preflight.nu`: parse the `Fabro-Run:` trailer of the matched commit and add it to duplicate-verdict rows.
- Evidence: preflight gave sha+subject but no run id; because the mandated closure-note form is `(run <run-id>)`, the planner burned an extra round trip on `git show a3f0834e9 --format=fuller`, whose ~4KB output is the squash-merge's embedded per-stage bullets, just to find `Fabro-Run: 01M5KGBR2BAS8BKYKG06PEVHMZ`.
- Effect: one fewer LLM round trip + one large tool output per duplicate-close pass.
- Tracker: new item. Search `preflight duplicate run id closure` returned 0.

**6. Add an English-response line to the loop planner prompt.**
- Change: `.fabro/workflows/loop/prompts/planner.md`: one line requiring English responses.
- Evidence: the planner's final stage output — the run's stdout record and stage Output render — was German ("`fabro-30a5` ist geschlossen: …") while the entire prompt/repo is English; that text feeds journals and downstream preamble consumers.
- Effect: consistent, greppable transcripts; no localization drift in Slack/journal records.
- Tracker: new item. Search `English responses prompt` found only fabro-eaa0, scoped to the *implementer* prompt — it does not cover the loop planner.

**7. (Minor) Clarify the title rule for non-claim terminal routes.**
- Change: same file, `prompts/planner.md`: state that `fabro_run_set_title` applies (or explicitly doesn't) on Already-landed/Tracker-empty exits.
- Evidence: the planner's reasoning trace spent a full deliberation block on this ambiguity ("I'll skip the title to obey the literal rule…") and skipped it, leaving the generic title "Maintain loop machinery one seed per run".
- Effect: removes wasted reasoning; kanban tiles stop showing generic titles on close-only runs.
- Tracker: new item. Search `run title` (8 rows) covers revisor title reliability (fabro-967e) and PR fallback titles (fabro-cf31) — neither covers the loop planner's exit-route title policy.

One thing I could not inspect: the actual PR #482 body/title content (only its creation event is in the stream), so recommendation 2's reviewer-noise claim rests on the diff summary, not the PR text.
