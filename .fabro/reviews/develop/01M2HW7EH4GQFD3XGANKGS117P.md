# Improve review — run 01M2HW7EH4GQFD3XGANKGS117P

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (32.3 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-15 07:20+0000 by revisor `fabro_ask`

---

All evidence is in. `scripts/verify.nu` line 45 confirms the nu-0.115 regex bug was already fixed (closed seed fabro-88f2, commit 29510edb0 — and this run's gate shows `lint-nu: green`), so the implementer's "via `just verify implementer`" PASS claim is legitimate. Here are the recommendations, grounded in this run's events, checkpoints, journals, and timings, ordered by expected impact.

**Run baseline (from run events/conclusion):** 31.8 min wall, $2.06 total; implementer 26.1 min / $1.86 (82% of wall, 90% of cost); planner 69.6s / $0.10; reviewer 52.4s / $0.09; gate 201.7s; zero cycles, zero retries, seed fabro-895d closed, PR #148.

---

**1. Make the reviewer's first blob action a bounded `nu str substring` read — seed fabro-9837 (open, exact match).**
What happened: the evidence capture was **63,342 bytes** (evidence@1 `output_bytes`), the graph budget is 24 KB and the reviewer node's `preamble_inline_max_kb=16` — so it arrived as a blob ref, and **all 4 of the reviewer's tool calls this run were blob recovery**: one `read_file` that truncated mid-file ("15870 bytes omitted") plus three `nu str substring` paging calls. It is the run's only journal painpoint. Change: rewrite the LARGE VALUES paragraph in `.fabro/workflows/develop/prompts/reviewer.md` to mandate `nu -c "open --raw <path> | str substring 0..20000"` before any `read_file`, and have the engine's blob marker state payload size (fabro-9837 part 1). Note the budget raises (fabro-8d2c 24→32, fabro-cf3e 16→32) would **not** have inlined a 63 KB capture — for captures this size, 9837 is the fix; the raises remain complementary for ~20–30 KB captures. Expected effect: one-shot evidence reads; removes the unread-blob Verification-blocked bounce (~2–4 min plus a second reviewer visit) that this run escaped only by luck.

**2. Replace the `sd ready` firehose with a top-N pick view — seed fabro-c3b4 (open, named by this run's planner).**
What happened: the planner's first call returned **200 seeds / 27,894 bytes with `stdout_truncated: true`** — the planner literally could not see the tail of its own candidate list, and journaled: "sd ready returned 200 seeds (at the --limit cap) — the backlog may exceed the cap; planner prompt's c3b4 top-N idea gains relevance." Change: `.fabro/workflows/develop/prompts/planner.md` — top-N view (`sd ready --priority high` / `--first 10`), full listing only when top candidates are unclaimable, plus a `created_since` bound on the `fabro_runs_list` in-flight check (c3b4's extension clause). Expected effect: ~25 KB less context per planning pass, no silently truncated candidate space.

**3. Require errored tool calls in the implementer journal — seed fabro-a23f (open, exact match).**
What happened: the implementer's tool stats show **edit_file 36 calls / 2 errors and shell 56 calls / 3 errors**, yet its journal reported `painpoints: []` and none of the observations name them. The friction signal (what the failed edits/shells were) reached nobody — same failure mode a23f was filed for. Change: add one line to the Journal section of `.fabro/workflows/develop/prompts/implementer.md`: an errored or timed-out tool call always goes in `observations`, naming the call and the workaround. Expected effect: loop friction self-reports on the run where it happens instead of requiring event-stream archaeology (what I just did).

**4. Batch planner reconnaissance into one shell call — seed fabro-55a7 (open).**
What happened: the planner's stale-basis verification ran as **8 sequential shell calls + 1 read_file** (rg file lists, rg 422/409, `read_file` supervisor lines 296–415, rg closed-wins, rg update_failures…) before claiming — 10 tool calls, 67.3s inference, in a stage that is pure reconnaissance. Change: `.fabro/workflows/develop/prompts/planner.md` — compose `sd ready + sd show + basis greps` into one shell invocation per phase. Expected effect: per 55a7's own basis measurement, ~40–60s and ~$0.06 saved per pass, and a shorter claim-to-dispatch window that narrows the duplicate-claim race.

**5. Expose `current_seed_id` in the `fabro_runs_list` projection — seed fabro-9372 (open).**
What happened: this run's `fabro_runs_list` output (event seq 36) lists 4 develop runs, **all carrying the identical generic goal with no seed id** — the planner had to reason "only this run itself is active" by self-exclusion, and would have needed the journal-grep fallback the moment any sibling was mid-flight. Change: engine-side, add the claimed seed id to the runs-list projection (the planner prompt already carries the degraded-mode fallback; this makes it unnecessary). Expected effect: the in-flight claim guard becomes deterministic instead of heuristic — this is the guard that prevents a second run re-implementing a seed whose PR sits in the gate (the fabro-22e4 class).

**6. Add a long-stage heartbeat notification — seed fabro-b769 (open).**
What happened: the implementer ran **26.1 minutes (82% of run wall, $1.86)** with zero external signal; this run's settings wire Slack only for `run.completed`/`run.failed`. Change: extend the notifications block in the develop workflow settings with a "stage exceeds N minutes" heartbeat. Expected effect: mid-run visibility into single-stage cost concentration while cancel is still possible — this run is the cleanest instance yet of the concentration b769 targets.

**7. Put the per-stage cost/time table and painpoint digest in the PR body — seed fabro-1409 (open).**
What happened: the fact that 90% of this run's $2.06 sat in the implementer stage, and the reviewer's blob painpoint, are visible only in the run conclusion/journal — not on PR #148, which is what the user actually merges. Change: engine PR postlude — append the per-stage timing/cost table plus a journal painpoint digest. Expected effect: cost concentration and loop friction reach the merge decision point without opening the run dashboard.

---

**One deliberate non-recommendation:** I checked whether the implementer's "`just verify implementer` PASS" could be false given mx-e606d5 (verify.nu broken on nu 0.115) — it is not: `scripts/verify.nu:45` now uses plain-string concatenation, fixed via closed seed fabro-88f2 before this run's base. Recommending it would have re-proposed closed work (the exact fabro-6c14 failure mode). No new seeds are needed above — every recommendation matched an open tracker seed.
