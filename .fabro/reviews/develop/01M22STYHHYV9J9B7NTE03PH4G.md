# Improve review — run 01M22STYHHYV9J9B7NTE03PH4G

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (2.5 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-09 10:08+0000 by revisor `fabro_ask`

---

All evidence below is from this run's event stream (planner@1, implementer@1, reviewer@1 transcripts, gate/script timings, conclusion billing), the worker log, and the run conclusion — run `01M22STYHHYV9J9B7NTE03PH4G`, seed `fabro-a18d`, all-green single-pass, 2m24s wall, $0.136 total (planner $0.061 / implementer $0.046 / reviewer $0.030).

## Recommendations, by expected impact

**1. Kill the `sd ready` firehose — planner context was dominated by a 24 KB seed listing it never needed past line 1.**
- What happened: planner's first call (event seq 31–33) ran `sd ready --assignee fabro --limit 200`, which returned **180 issues / 24,314 bytes** (`stdout_truncated: true`). It then picked candidate #1 (`fabro-a18d`) and never used the remaining ~175 lines — but they sat in context for all 7 planner LLM turns (conversation tokens grew 3.5k → 13.7k). The planner ended up the **most expensive stage: 45% of run cost, 36.4s inference**.
- Change: `.fabro/workflows/develop/prompts/planner.md`, step 1/2 of "Plan the next seed" (the sd command table): make the first call `sd ready --assignee fabro --limit 10` and widen to `--limit 200` only when the top slice is unusable (all in-flight / stale). This is exactly open seed fabro-c3b4; this run is its evidence.
- Expected effect: ~22 KB less planner context per run → roughly 30–40% off planner input tokens and several seconds of latency on every develop run.

**2. Scoped reads + conditional spec re-fetch for implementer/reviewer when the brief pins anchors.**
- What happened: the brief pinned "steps 6 and 7 … lines ~42-46", yet implementer@1 opened `sed -n 1,80p` of `planner.md` (12.6 KB, seq 85–87), then a separate `grep -n` to re-locate line 43 (seq 93), and re-ran `sd show fabro-a18d` (seq 84) although the brief already contained the seed body verbatim. The reviewer repeated the pattern: `sed -n 30,55p` (8.2 KB, `stdout_truncated: true`, seq 152–154) to verify a 3-line hunk at 41–47.
- Change: `.fabro/workflows/develop/prompts/implementer.md` step 1 ("Re-read the seed requirements") — make `sd show` conditional on the brief being thin (open seeds fabro-a67f/fabro-4881), and add "when the brief pins line anchors, read exactly `sed -n '<anchor>±5p'`" (open seed fabro-645d). Mirror one line in `reviewer.md` (open seed fabro-9a43).
- Expected effect: 2–3 fewer tool turns and ~20 KB less context per pass; on this run that's ~10–15s and ~15% off the implementer+reviewer stages.

**3. Share a prompt-cache prefix across stages — every stage's first LLM call paid full input price.**
- What happened: `cache_write_tokens = 0` on every call in the run; planner's first call (seq 30) had `cache_read_tokens: 0` on 12.3k input, same for implementer (seq 83) and reviewer (seq 151). The system prompt + AGENTS.md memory (~7k tokens) is byte-identical across all three agent stages yet was re-paid each time — the three "first calls" alone cost ~$0.055 of the $0.136 total.
- Change: engine-side, open seed fabro-944d ("shared prompt-cache prefix across stages", `lib/components/fabro-workflow`); stage sessions should reuse the cached prefix from the previous agent stage's session since the preamble head is stable.
- Expected effect: ~25–35% input-token cost cut per develop run; zero behavioral risk (pure billing path).

**4. Make the "workaround you performed is an observation" rule apply to the fs_hide shell detour — the implementer journaled "none" over a real workaround.**
- What happened: implementer@1's reasoning trace (seq 83) says "it's under `.fabro/`, so file tools are blocked; use shell" and it edited via a python3 heredoc — a textbook workaround on a platform-targeting seed — yet its journal `observations` answer was `["none"]` (seq 111). The journal is the loop's only feedback pipe to the improve workflow; this run contributed zero implementer signal.
- Change: `.fabro/workflows/develop/prompts/implementer.md`, journal section: add one sentence — "on platform-targeting seeds, the fs_hide shell detour you performed (including which tool you substituted) is an observation; 'none' is wrong when you worked around a denial."
- Expected effect: the improve loop stops losing the one recurring friction fact every platform-targeting seed re-discovers.

**5. Downgrade the by-design `preamble_allow_keys` absence WARN — it fired on a run where the key can never exist.**
- What happened: the run's only worker-log warning (10:02:17) is `preamble_allow_keys entry absent from context node=implementer key=output.gate_known_bug_hits`. That key is produced solely by the `gatebounce` node, which only runs on gate-red; this was an all-green run, so the warning is structural noise on every happy path.
- Change: engine lifecycle/fidelity logging (`lib/components/fabro-workflow`, alongside open seed fabro-8275): emit info, not warn, when the absent key's producing node was never visited this run.
- Expected effect: warn-level logs become trustworthy again — a real envelope violation won't sit in a sea of by-design warnings.

**6. Put the per-stage cost/time table and journal digest on the PR — the user merging PR #82 currently sees none of this run's facts.**
- What happened: PR #82 was created 12s after closeout (seq 181) with a title-only postlude; the run summary still shows `pull_request.state: null` (no live state relayed back), and the interesting facts — planner 45% of cost, gate green in 4.4s with "no crates touched", reviewer byte-for-byte verification — live only in the run dashboard. Open seed fabro-1409 tracks exactly this; this run is a clean instance.
- Change: PR postlude composition in the engine (same area fabro-6a5a/fabro-1409 target): append the conclusion's per-stage timing/cost table plus a one-line digest of journal observations.
- Expected effect: at merge time the user sees cost distribution and what review actually verified without opening the dashboard.

One caveat on what I could not verify: no merge/auto-merge event for PR #82 appears in the stored event stream (it ends at `run.completed`, 10:04:04, two seconds after PR creation), so I made no recommendation about auto-merge behavior from this run — the known auto-merge concerns are already captured in open seed fabro-6a5a.
