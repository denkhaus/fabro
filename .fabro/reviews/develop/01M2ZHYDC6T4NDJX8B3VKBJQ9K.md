# Improve review — run 01M2ZHYDC6T4NDJX8B3VKBJQ9K

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (2.1 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-20 14:08+0000 by revisor `fabro_ask`

---

All evidence is in. Here are the recommendations for run `01M2ZHYDC6T4NDJX8B3VKBJQ9K`, ordered by expected impact. Context: this was a 2m08s "Already landed" run — `tracker_guard` (0.35s) → `preflight` (11.7s) → `planner` (97.1s, $0.192, 8 tool calls) → exit, closing fabro-9114 via superseded-close; implementer/tester/reviewer never ran (all facts from run events and the planner journal).

---

**1. Count direct no-PR-suffix fix commits as landed — seed fabro-679a (open, P1)**
- **What happened:** The planner hand-adjudicated fabro-9114's landed-ness across two extra LLM rounds + two shell calls (`git log --grep fabro-9114`, `git branch --contains 1e239bc8b`, events seq 68–79, ~$0.056 / ~30s of the 97s planner pass). The fix commit `1e239bc8b` is a direct single-parent commit on `origin/denkhaus` with **no `(#n)` PR suffix** — exactly the class fabro-679a says `dup-run-check.nu` drops into `$other`, so even with the seed in the preflight table the verdict would have been `clean`.
- **Change:** `.fabro/scripts/dup-run-check.nu` landed-set builder — fold seed-id-referencing single-parent base-branch commits into the landed set via `classify-filed` (as the seed specifies).
- **Expected effect:** This run class (stale tracker + landed direct commit) collapses from a ~97s/$0.19 planner lap to a ~12s zero-LLM preflight verdict with a `duplicate` row.

**2. Gate the preflight `in_flight` marker on run/PR terminality — seed fabro-ab93 (open, P1)**
- **What happened:** The preflight table marked fabro-8e13 `in_flight: true` via run `01M2ZEHCA127XFV2CMCVSEZDPJ` — but that run is **terminal-succeeded with no PR** (visible in the `fabro_runs_list` output, seq 49). The planner burned a long deliberation round reconciling the contradiction (seq 52 reasoning trace), skipped a priority-1 seed on a stale marker, and journaled "next planner should reconcile that branch" — meaning every following run re-hits this until fixed.
- **Change:** `.fabro/workflows/develop/scripts/planner-preflight.nu`, the `in_flight` arm (~lines 206–208): mark `in_flight` only when the run is non-terminal OR its PR state is open/unknown-and-branch-present.
- **Expected effect:** `in_flight` becomes trustworthy; fabro-8e13 stops being perpetually skipped, and planners stop spending a reasoning lap per run on stale markers (fabro-ab93's basis run measured ~60s/6 rounds for the same class; this run is fresh evidence).

**3. Give the planner a bounded `sd ready` view — seed fabro-c3b4 (open; its scope is exactly this)**
- **What happened:** The planner ran `sd ready --assignee fabro --limit 200 --format json` (seq 45) — **inventing `--format json`**, which the PROJECT_FACTS command table doesn't sanction for `sd ready` — and received **287,482 bytes**, double-truncated (`original token count: 71885 … 257540 bytes omitted`, seq 47). The full bodies of ~200 seeds then sat in the conversation (~49k conversation tokens at peak) even though finalists get re-fetched via `sd show` anyway (seq 54).
- **Change:** `.fabro/workflows/develop/prompts/planner.md` step 1 plus the command table in `.fabro/workflows/develop/prompts/project-facts.md`: sanction a compact top-N form (`sd ready --assignee fabro --first 10`, no descriptions), full listing only as fallback — precisely fabro-c3b4's demand.
- **Expected effect:** ~250KB of truncated firehose per planning pass disappears; the planner stops inventing flags on a command whose table row doesn't have them.

**4. Bound `fabro_runs_list` with `created_since` + self-exclusion — seed fabro-6b58 (open)**
- **What happened:** One `fabro_runs_list` call with no bounds (seq 48) took **~15.5s** (14:01:59.27→14:02:14.77) and returned **116 runs**, forcing the planner to manually deduce "Only this run is non-terminal" (seq 52). I verified planner.md:32 still carries no `created_since` bound — the regression fabro-6b58 already documented (its basis run: 16 runs/14.5KB; this run: 116 runs).
- **Change:** `.fabro/workflows/develop/prompts/planner.md:32` (step 4): `fabro_runs_list workflow "develop" created_since ≈ 48h`, plus one line excluding the run's own id from the in-flight set.
- **Expected effect:** ~15s tool latency and a large context blob per planner pass removed; the self-run confusion class disappears.

**5. Raise preflight candidate depth 5 → 10 — new seed needed.**
- **What happened:** The planner's own journaled painpoint: the preflight table covered only the top 5 `sd ready` seeds (fabro-8e13…fabro-d4c6) while the actionable seed fabro-9114 sat further down; landed-ness had to be adjudicated by hand. The depth knob is `--top: int = 5` at `.fabro/workflows/develop/scripts/planner-preflight.nu:227` (default; documented at line 58).
- **Change:** Default `--top` to 10. Preflight cost scales ~2.3s/candidate (11.7s for 5), so ~23s — still far under the 120s node timeout and cheaper than one LLM round it replaces. Note: only pays off in combination with rec 1 (a direct-commit fix at rank 6+ still reads `clean` without it).
- **Expected effect:** The two-branch rule gets deterministic verdicts for lower-ranked candidates; the rec-1 saving materializes even when the stale seed isn't top-5.
- **New-seed justification:** fabro-ead4 (non-top duplicate candidates) was about extending the closure arm and is closed 2026-09-19; no open seed owns the candidate-table depth itself.

**6. Stop opening PRs for bookkeeping-only diffs — new seed needed.**
- **What happened:** The run's entire diff is journal + one seed-row edit (2 files, +5/−1), yet PR #327 was opened ("Close seed fabro-9114 as superseded…", seq 99) with auto-merge armed — a full CI gate and merge on `denkhaus` for tracker bookkeeping. With 116 develop runs and counting, this is steady PR/CI churn and history noise per no-cycle run.
- **Change:** Engine-side PR-open path (or a per-graph `run.pull_request` condition): skip PR creation when the final diff touches only `.fabro/journal/**` and `.seeds/**` — push the branch and let the next real merge sweep it, or fold into the revisor's sync.
- **Expected effect:** No-op PRs stop accumulating on the merge-target branch; the PR list becomes signal again for the user watching the loop.
- **New-seed justification:** grepped the tracker — fabro-2e66 covers surfacing PR-pipeline degradation, not suppressing tracker-only PRs; nothing else touches PR-open policy for bookkeeping diffs.

**7. Silence the per-pass `context_allow_keys dropped: output.planner` notice — new seed needed.**
- **What happened:** Warn-level run.notice at seq 90: the planner's `context_updates` carried the engine's response-dedup payload (`output_key: output.planner`, the fabro-b907 mechanism), which isn't in the planner node's `context_allow_keys` (`current_seed_id,current_seed_title,current_seed_brief,review_verdict,journal`), so the fabro-900e lint drops it **visibly on every planner pass** — warn noise that dilutes the lint's real signal (actual drift).
- **Change:** Either declare the dedup output key in the planner node's `context_allow_keys` in `.fabro/workflows/develop/workflow.fabro`, or exempt engine-generated response-dedup keys from the drop notice.
- **Expected effect:** One fewer spurious warn per run; the envelope lint stays clean for genuine contract drift.
- **New-seed justification:** fabro-b907 (response dedup) and fabro-900e (envelope declaration) are both closed and predate this interaction; no seed covers the engine-internal key being flagged by its own lint.

---

**Not worth a recommendation:** the run's core flow worked as designed — `tracker_guard` fast-cleared in 0.35s, the planner correctly took the fabro-d183 branch-(a) superseded-close with the chained note-append+close in one shell call (seq 80), and the fabro-83df report-only preflight correctly left the closure decision with the planner. The friction was data quality reaching the planner (recs 1–5), not the graph's routing.

Could not inspect: PR #327's post-creation state (merge/CI outcome) — the event log ends at `run.completed`/`pull_request.created` with PR state null, and I have no GitHub access from this analyst scope.
