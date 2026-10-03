# Improve review — run 01M2MTN5MJ17SA67M8N82HZMSE

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (2.9 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-16 10:15+0000 by revisor `fabro_ask`

---

All recommendations below are grounded in this run's events/transcripts (run `01M2MTN5MJ17SA67M8N82HZMSE`, seed `fabro-a67f`, a 2-line reword of `.fabro/workflows/develop/prompts/implementer.md`; totals: 158 s active, $0.209, planner 55 s/$0.097, implementer 62 s/$0.075, reviewer 18 s/$0.036, gate 3.6 s). I checked every cited seed id against `.seeds/issues.jsonl` — all are open and fabro-assigned; **no new seeds are needed**, each rec names an existing one.

**1. Ship `fabro-9372` (+ `fabro-4bb7`): expose `current_seed_id` (and backfill `pull_request.state`) in the `fabro_runs_list` projection.**
Evidence: planner seq 33 fetched 16 runs / 14.4 KB whose `goal` strings are all the identical generic text; seq 36 shows it hand-classifying in-flight runs ("PR state null but status succeeded — not in flight") — pure LLM re-derivation of data the engine already holds. Change: engine run projection (the `fabro_runs_list` tool surface), interim per the seed's planner.md journal fallback line. Effect: removes 1 reasoning round + fragile heuristics from **every** develop claim, and closes the double-pick window the seed's fresh-evidence note documents.

**2. Ship `fabro-ab38`: post-merge sweep marking satisfied sibling seeds superseded.**
Evidence: this run's planner discovered seed part (a) was already landed by `fabro-6e7f` — ~30 s and two grep passes (seq 43–50) proving staleness, plus a 2.5 KB `sd update --description` correction — and then left "fabro-4881 overlaps; next planner should stale-basis-check it" **only in this run's journal**, which the next run's planner never sees (its `journal` key starts empty). Change: extend `closeout.nu` / improve workflow per the seed. Effect: overlaps like `fabro-4881` get closed deterministically at merge time instead of being re-proven (or re-implemented) by every subsequent planner.

**3. Ship `fabro-7b2a`: cap the implementer step-4 rule, move run citations to footnotes.**
Evidence: implementer@1 spent 62.0 s inference against 1.0 s tool time for a fully-specified 2-line edit, and burned two shell calls re-reading the step-1/step-4 blocks (7.7 KB each, seq 90–98) just to orient inside its own ~14 KB prompt; it was 36% of run cost. Change: `.fabro/workflows/develop/prompts/implementer.md` step 4 per the seed (including its REQUIRED FIX contradiction note). Effect: shorter implementer prompts every run — the largest LLM stage gets cheaper and faster for exactly this trivial-seed class.

**4. Ship `fabro-c3b4` / `fabro-66bc` (+ `fabro-55a7`): top-N `sd ready` view and batched reconnaissance.**
Evidence: planner's first call poured the full 200-seed firehose — 27,865 bytes, `stdout_truncated: true` (seq 30) — and it then used only the top High candidate; 7 separate tool calls across 7 LLM turns made the planner 37% of run wall time. Change: planner.md command table → `sd ready --first 10`/priority-sorted, and one batched recon shell call. Effect: ~27 KB less context per planning pass, fewer turns, and a shorter claim-to-dispatch window that narrows the duplicate-claim race.

**5. Ship `fabro-c080`: re-resolve stage prompt files from the run branch tip.**
Evidence: this run edited `implementer.md` (commit `b71efe9`), yet the implementer@1 and reviewer@1 stage prompts (seq 77, 151) still contained the **old** step-1 text — the reviewer approved the new wording while executing under the superseded instruction (benign only because it cross-checked the worktree). Change: engine run-spec prompt resolution per the seed. Effect: prompt-editing seeds — half this backlog — can never be judged against stale copies of the very file they changed.

**6. Ship `fabro-45bf`: compute progress from unique completed nodes.**
Evidence: implementer@1 header read "Pipeline progress: 0 of 7 stages completed" with the planner already done (seq 77); reviewer@1 read "2 of 7" with five nodes done (seq 151). Change: engine progress projection + stage-preamble header (both surfaces, per the seed's scope update). Effect: honest mid-loop numbers for agents and the user-facing run view; this run adds two fresh instances.

**7. Ship `fabro-41b1`: make PR-body generation non-strict.**
Evidence: worker log 10:04:57 — `PR content structured generation failed; retrying once without strict JSON output … model did not return a JSON document` — the strict-parse failure hit this run and the retry masked it. Change: PR postlude (`lib/components/fabro-workflow/src/pipeline/publish.rs` path per the seed). Effect: no wasted generation round + no masked failures in the publish path that every green run depends on.

**8. Ship `fabro-9ef9`: give the evidence node its own output context key.**
Evidence: `command.output` was written three times this run — tester (blob `ae010c70…`, checkpoint seq 140), evidence (`b7b16ac0…`, seq 148), closeout (`c65888e0…`, seq 182) — textbook last-writer-wins; it didn't bite only because the gate was green. Change: key the evidence command node to `evidence.output` in `workflow.fabro` + `evidence.nu`. Effect: on a red-gate bounce the gate log stays readable in context for the reviewer instead of being clobbered by the capture.

**9. Ship `fabro-a701` + `fabro-8275`: log-severity hygiene.**
Evidence: worker log shows 6 ERROR lines for the absent optional `.codex/instructions.md` (two per agent session × 3 sessions) and 1 WARN for the by-design absence of `output.gate_known_bug_hits` on a green first visit — noise in exactly the warn+error view an analyst uses. Change: agent session init (`fabro-a701`) and the fidelity warn (`fabro-8275`). Effect: the warn/error channel stops crying wolf; real failures (like rec 7's) surface immediately.

Not inspected: gate-log blob contents (blob refs aren't readable with my tools), the PR #171 post-merge events after run completion (last event 10:13:08), and reviewer tool-call detail beyond its single 52 ms shell call. Reviewer economy (`fabro-50c9`) was deliberately **not** recommended — this run's reviewer was already lean (18 s, 1 tool call).
