# Improve review — run 01M2YAJMSPY766DS0AGW1859CQ

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (2.0 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-20 02:43+0000 by revisor `fabro_ask`

---

# Recommendations for the develop workflow — grounded in run 01M2YAJMSPY766DS0AGW1859CQ

**What this run actually was** (from run events and conclusion): a 100%-bookkeeping lap. `tracker_guard` (0.44 s) requeued stale seed fabro-ae57 → `preflight` (4.43 s) produced a healthy verdict table → the planner (92 s wall, 76.4 s inference, 9 tool calls, **$0.209 = 100% of run LLM spend**) re-adjudicated and superseded-closed fabro-ae57, routed "Already landed", exited. No implementer/tester/reviewer ran; PR #306 carries only the journal + tracker diff (2 files, +5/−1). Seed-id checks below are against the tracker listing captured in this run's `sd ready` output (run events, seq 46–47).

---

**1. Make closeout verify the close actually landed** — error handling; change `nu .fabro/workflows/develop/scripts/closeout.nu` to re-check `sd show <id>` after `sd close` and hard-fail (soft exit, seed stays visibly open) or auto-retry when the seed is still open.
*Evidence:* the planner's own journal painpoint — run 01M2Y8QYY already verified fabro-ae57 and merged as c774f9e3 (PR #304), "but its close never landed, so tracker_guard requeued it and this run had to re-adjudicate and re-close." This entire run ($0.21, ~2 min, a PR) existed only to repair that silent miss.
*Expected effect:* eliminates whole duplicate re-adjudication runs of this class.
**New-seed justification:** no open seed covers close-verification — fabro-fbed delivered the closeout node, fabro-7aac/fabro-534e re-file *disclosed deferred actions*; none detects a close that silently failed to land.

**2. Requeue claims immediately when the claiming run is terminal-failed** — graph/script design; in `.fabro/workflows/develop/scripts/tracker-guard.nu`, drop the 6 h age gate when the claiming run is already terminal-failed with no PR.
*Evidence:* the two **top-priority** candidates (fabro-af22 is a *High* bug, fabro-b03f) were pinned `in_flight` by run 01M2Y71ZR33, which **failed at 01:42 with no PR** (preflight table + `fabro_runs_list` output, run events seq 49). The guard requeued only fabro-ae57; af22/b03f stay pinned until the 6 h stale window (~07:32) — the planner journaled exactly this ("if that run is dead, a human or tracker_guard staleness window may need to release them"). At the observed half-hourly run cadence (runs created 02:33, 02:01, 01:31, 01:01…), ~10 runs will work lower-priority seeds while the High-priority one idles.
*Expected effect:* priority inversion capped at one run instead of ~5 hours.
**New-seed justification:** fabro-6b58 bounds the *run-list call*, and the requeue arm (fabro-d9f7) shipped with the fixed 6 h threshold — no open seed keys requeue to the claiming run's terminal-failed status.

**3. Bound the in-flight `fabro_runs_list` call and expose `current_seed_id` in its projection** — tool usage; pass `created_since` (e.g. 48 h) and add the seed-id field consumed by the planner prompt's step 4.
*Evidence:* the single `fabro_runs_list` call took **14.0 s** (02:33:51.435→02:34:05.470) and returned **104 runs** — 92% of the stage's 15.3 s tool time — forcing the planner to reverse-engineer seed ids from goals and manually self-exclude (this run was row 1 of the listing).
*Expected effect:* ~10 s + one reasoning round saved on *every* planning lap.
**Seed:** **fabro-6b58** (created_since window + self-exclusion), with **fabro-9372** (expose `current_seed_id` in the projection) as the complementary half.

**4. Stop the `sd ready` firehose — trust the preflight table** — prompting + tool usage; edit `.fabro/workflows/develop/prompts/planner.md` (step 1/4) to treat `output.preflight`'s ranked candidate table as the candidate source and skip `sd ready` when the table is healthy.
*Evidence:* the planner ran `sd ready --assignee fabro --limit 200` pulling **28,359 bytes / 200 rows** (event record flagged `stdout_truncated: true`) — yet every candidate it acted on (af22, b03f, ae57) was already in the preflight table with verdicts and in-flight flags.
*Expected effect:* one fewer 28 KB payload in context (~7 k conversation tokens of the ~49 k the planner carried) and one fewer LLM turn per lap.
**Seed:** **fabro-c3b4** ("Use a top-N sd ready view in the planner instead of the full firehose") — this run is its direct evidence.

**5. Batch planner reconnaissance into one shell call** — prompting; apply the fabro-55a7 pattern to the landed-ness adjudication sequence in `planner.md`.
*Evidence:* the planner issued 9 tool calls across 9 sequential LLM turns; measured first-token waits were 5.5/5.9/6.3/7.3/5.4/11.0/7.8/3.3/2.9 s ≈ **61 s of the 76.4 s "inference" time was TTFT waiting**, not thinking. The ae57 adjudication (`sd show` → grep+`git log` → `sed` → `git fetch`+branch-contains → `git log --grep 269d`) was 5 round-trips that chain into 1–2 calls.
*Expected effect:* ~30–40 s off every planning lap at zero information loss.
**Seed:** **fabro-55a7** ("Planner: batch reconnaissance into one shell call (sd ready + sd show + base-branch grep)").

**6. Flag incidentally-satisfied seeds in the preflight (cross-seed fix references)** — tool/graph design; extend `.fabro/workflows/develop/scripts/planner-preflight.nu` to sweep a candidate's named target file against recent merged run diffs, not just grep commit subjects for the seed id.
*Evidence:* preflight returned verdict **`clean`** for fabro-ae57 while its fix (2dd43d39, landed via fabro-269d) was already in `origin/denkhaus` — the id-grep missed it because the fix commit references fabro-269d, not fabro-ae57. The planner then burned 5 calls/turns re-deriving landed-ness by hand (run events seq 53–79). `filed_only_matches=1` (the c774f9e3 verify commit) was the only hint.
*Expected effect:* the planner adjudicates from one table row instead of a 5-call investigation; prevents "clean"-verdict surprises on sibling-satisfied seeds.
**Seed:** **fabro-ab38** ("Sweep open seeds against the merged run diff and mark satisfied siblings superseded") — same class, deterministic-sweep direction.

**7. Make bookkeeping-only runs visible as such** — UX; annotate `run.completed` (this run fires Slack `#dev-fabro` per settings) and consider the gate-skip for tracker-only PRs.
*Evidence:* this run's terminal notification is indistinguishable from a real dev cycle, and PR #306 (auto-merge enabled) carries only journal + tracker churn — yet the run's `status: succeeded(completed)` reads like shipped feature work. (Not inspected: the actual Slack payload and PR #306's CI runs — outside this run's events.)
*Expected effect:* humans triage notifications correctly; CI budget not spent on tracker-only diffs.
**Seeds:** **fabro-b1d3** (Slack annotation for bookkeeping-only runs) primary; **fabro-9495** (skip project gate on loop-asset-only run PRs) for the CI half.

---

**Not worth acting on from this run:** reasoning-effort tuning (already `low`; only 1,199 reasoning tokens), preamble budget (planner context peaked at 5.8% of the 1 M window), and the preflight node itself (healthy 4.4 s table — the fabro-a32f/fabro-2ade fix is working). One cosmetic engine nit observed: `context_update_dropped: output.planner` warn at seq 99 — the planner node's `context_allow_keys` omits its own response key; harmless on exit paths but worth folding into fabro-meta-c454's context-contract cleanup if that seed is taken.
