# Improve review — run 01M2N1G834RTCSZNX7EK40G3SE

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (3.8 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-16 12:12+0000 by revisor `fabro_ask`

---

All facts below come from this run's event history (`fabro_run_events`), its conclusion/usage block (`fabro_run_get`), and the seed listings the planner itself fetched this run (seq 30–31). Run shape: seed **fabro-3c9d** claimed as *verification-only*, zero code changes, all 6 stages green first-pass, PR #177 auto-merged. Totals: **206 s wall, $0.228, 77.3k input tokens**; planner 101.7 s / $0.121 (53% of cost), implementer 57.1 s / $0.064, reviewer 26.7 s / $0.043, tester 3.7 s, evidence 0.26 s, closeout 0.27 s. Final diff: 2 files, +8/−1 (journal + tracker status flip only).

## Recommendations, by expected impact

**1. Add a first-pass verification-only fast path: planner → evidence → reviewer (skip implementer + tester).**
- Change: in `.fabro/workflows/develop/workflow.fabro`, add `planner -> evidence [label="Verification-only", condition="preferred_label=\"Verification-only\""]` and a matching output variant in `.fabro/workflows/develop/prompts/planner.md`; evidence.nu's churn-only path already handles a no-diff tree (ran clean here, 0.26 s).
- Grounding: the planner *already* verified all three criteria pre-claim (seq 43–75: grep of `diff-sort-key`, budget constants, diff-walk), the implementer re-verified the same three facts (5 shell calls, 57 s, $0.064), and the reviewer verified them a third time (3 shell calls). Triple verification of identical line-level facts.
- Effect: −60 s and −$0.07 (≈30% of cost) on every branch-(b) verification-only run.
- Seed: **new seed justified** — fabro-ff7a covers *no-change resubmits* only; nothing covers the *first* verification-only claim, which this run proves burns the same full cycle.

**2. Ship fabro-9f97 (skip PR creation on journal-only run diffs).**
- Grounding: this run minted PR #177 (squash, auto-merge) for a diff containing only `.fabro/journal/…jsonl` + the `.seeds/issues.jsonl` status flip — zero product change reaching `denkhaus`.
- Effect: eliminates no-op PRs and merge noise per bookkeeping run.
- Seed: **fabro-9f97** (open, High). Implementation must preserve tracker-close delivery to the merge-target branch (`.seeds` is git-native), per the tension already noted in open fabro-9495; pair with fabro-b1d3 for the Slack payload annotation.

**3. Put the planner on a token/turn diet (top-N `sd ready`, batched recon, bounded runs_list, scoped memory).**
- Grounding: planner = 53% of run cost and 49% of wall, 10 serial tool calls, 33k input tokens. Its first call returned the full firehose — 200 seeds, 28,024 bytes retained (seq 30) — while it only needed the top High candidate; `fabro_runs_list` returned 19 runs / 17 KB back to 2026-09-14; every agent stage also re-loads the full 25,271-byte `AGENTS.md` (seq 21, 97).
- Change: prompts/planner.md + the sd invocation (top-N priority view), one batched recon call, `created_since` bound; engine-side memory scoping.
- Effect: ~30–50% planner cost cut (~$0.04–0.06/run) and 40–60 s wall from fewer LLM turns (observed 5–13 s latency per turn).
- Seeds: **fabro-c3b4** (or its duplicate **fabro-66bc**), **fabro-55a7**, **fabro-6b58**, **fabro-9588** — all open, all directly evidenced by this run's transcripts.

**4. Capture verification-only check outputs into the evidence pipe.**
- Change: `.fabro/workflows/develop/scripts/evidence.nu` — when seed-work is empty and the brief is verification-only, emit the implementer's per-criterion check commands + their outputs as a section.
- Grounding: this run's capture was 1,774 bytes of "(no seed-work files to diff)", which *forced* the reviewer to re-derive everything with tools — the exact anti-pattern closed seed fabro-50c9 banned for diffs ("judge from the capture; tools only for what the capture cannot show").
- Effect: reviewer shell calls 3→0, review ~27 s→~15 s, and removes implementer/reviewer verification drift risk.
- Seed: **new seed justified** — open evidence-pipe seeds (fabro-750c gate tail, fabro-b798 diff-stat map, fabro-8cef blob format) all target diff-carrying captures; none cover verification-only runs.

**5. Require labeled segments in compound probe commands (planner prompt one-liner).**
- Change: one line in `.fabro/workflows/develop/prompts/planner.md`: "When chaining probes, prefix each with `echo '=== <label> ==='`."
- Grounding: seq 55–72 — the planner chained `git log --grep 3c9d` (empty) with `git log -- evidence.nu` (54b6fcf), misattributed the output, briefly believed the sync commit referenced the seed, then burned 3 extra calls (~30 s) on `git log --follow` / `branch --contains` / `git show` to rule out branch (a).
- Effect: prevents this ~30 s misdiagnosis class per planner pass.
- Seed: **new seed justified** — probe-discipline seeds fabro-b6f9 and fabro-8d4c cover redirection/quoting and read shape, not compound-output labeling.

**6. Verify the pipeline-progress header fix (apparent residual miscount).**
- Grounding: despite the dedupe fix (#175, in this run's base d9ee233), the implementer prompt read "Pipeline progress: 0 of 7 stages completed" *after* planner completed (seq 95), and the reviewer prompt read "2 of 7" with 4 non-meta nodes completed — fabro-45bf was closed 40 minutes before this run claiming both surfaces fixed.
- Change: engine progress projection (the `lib/` workspace surface fabro-45bf named); re-check what the mid-run header counts.
- Effect: honest mid-run progress numbers — the run snapshot's own "6 of 7" only becomes correct at termination.
- Seed: **new seed justified** — fabro-45bf is closed; this run is post-fix counter-evidence, so a regression-verification seed is warranted rather than reopening blind.

Not recommended from this run: gate/error-handling changes — zero retries, zero reds, gate green in 3.7 s ("no crates touched"); the cycle-guard and gatebounce nodes were never exercised, so there is no this-run evidence to ground changes there.
