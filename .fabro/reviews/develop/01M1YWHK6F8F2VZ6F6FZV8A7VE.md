# Improve review — run 01M1YWHK6F8F2VZ6F6FZV8A7VE

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (2.5 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-07 21:38+0000 by revisor `fabro_ask`

---

All grounding below is from this run's events, stage records, and the worker log (run 01M1YWHK6F8F2VZ6F6FZV8A7VE, seed fabro-2254, 7 stages, ~124 s wall, $0.136, zero retries — a clean single-pass run). Ordered by expected impact; the things that already worked well (touched-crates gate at 5.4 s "no crates touched", evidence capture at 367 ms, gatebounce never needed) get no recommendations.

## 1. Make checkpoint metadata snapshots async — ~15 s/run, ~12% of wall time
**Evidence:** 8 `metadata.snapshot.completed` events totaling **14,979 ms** (init 2,039; planner 1,695; implementer 1,822; tester 1,701; evidence 1,881; reviewer 2,047; closeout 1,699; finalize 2,095), serialized between every stage — including after command nodes (tester/evidence/closeout) whose only delta is one journal line. Payload grew 90 KB → 349 KB across the run.
**Change:** Engine checkpoint pipeline (already scoped by open seeds fabro-cf03 "async or branch-point-only" and fabro-c2ca "skip non-agent stages"): snapshot concurrently with the next stage's session warm-up, and skip snapshots whose only diff is `.fabro/journal/*.jsonl`.
**Effect:** −10–15 s on every run with no semantic loss; on a 2-minute run that's the single biggest wall-time win.

## 2. Replace the `sd ready` firehose with a priority top-N view — planner is 35% of run cost
**Evidence:** Planner's first tool call returned **160 ready seeds / 21,288 bytes, `stdout_truncated: true`** (event seq 33); that listing entered the billed conversation. Planner: $0.0476 of $0.1356 total, 18.8k input tokens — the most expensive stage — for picking the top-priority seed, which was first in the list.
**Change:** `.fabro/workflows/develop/prompts/planner.md` sd table (open seeds fabro-c3b4/fabro-e4fa): since `sd ready` already sorts by priority, render only the top ~15 plus a count (`... --limit 200 | head -15` shape or a `--top` flag), with the full list reserved for the fall-through path.
**Effect:** −~5k tokens and one less scrolled context per planner pass, and lower mis-pick risk from truncation; roughly 20–25% planner cost cut on runs like this one.

## 3. Gate the implementer's `sd show` re-fetch on brief quality — one wasted LLM round-trip per pass
**Evidence:** Implementer step 1 mandates "Re-read the seed requirements from `sd show`", so it ran `sd show fabro-2254` (seq 80–85) even though the planner's brief — written 40 s earlier — contained the spec verbatim. The implementer's reasoning traces (seq 88, 94) never use anything from the re-fetch that wasn't in the brief.
**Change:** `.fabro/workflows/develop/prompts/implementer.md` step 1 (open seeds fabro-4881/fabro-a67f): make the re-fetch conditional — "only when the brief is thin, ambiguous, or a re-plan with feedback."
**Effect:** One fewer tool call + one fewer model turn per implementer pass (~4–8 s, ~$0.01); matters more on bounce passes where turns multiply.

## 4. Prime a shared prompt-cache prefix across stages — cache_write_tokens is 0 everywhere
**Evidence:** Every billing record shows `cache_write_tokens: 0` while cache-reads work *within* a stage; each of the 3 agent stages opened a fresh session and re-paid a cold ~11.5k-token first turn (planner $0.0163, implementer $0.0167, reviewer similar) for identical payload: 24 KB `AGENTS.md` memory (~5.8k tokens), tool defs (~1.7k), system prompt (~1.1k).
**Change:** Engine agent-session layer (open seed fabro-944d): reuse/warm the provider cache prefix (memory + tools + system prompt) across stage sessions in a run.
**Effect:** ~25–30% input-token reduction per run at current prompt sizes; grows linearly with agent-stage count.

## 5. Stop the false-positive WARN for `gate_known_bug_hits` — it's the only warning this run produced
**Evidence:** Worker log has exactly one line: `WARN ... preamble_allow_keys entry absent from context node=implementer key=output.gate_known_bug_hits` — expected by design on first visits and green runs (the key exists only after a gate-bounce, which never happened here).
**Change:** Engine fidelity/lifecycle check (mirror of open seed fabro-8275): log at info unless the run actually took a `tester → gatebounce` edge.
**Effect:** Zero-noise warn channel, so a real preamble regression in an implementer pass isn't buried under per-visit expected warnings (important if warnings ever feed Slack/alerting).

## 6. Point the reviewer's phrase checks at `read_file offset/limit`, not grep alternations
**Evidence:** Reviewer journal (event seq 174, observation 2): "First grep alternation returned only the line-25 match in visible output, requiring a second targeted grep for the carve-out clause — minor tool-output truncation." That's an extra model round-trip (~3–4 s inference of the reviewer's 23.7 s) for a verification the diff line numbers had already localized to lines 25 and 78.
**Change:** `.fabro/workflows/develop/prompts/reviewer.md` tool-guidance section: when the diff names exact line numbers, verify with `read_file offset/limit` on those lines (or pass explicit `max_results` to grep) instead of multi-pattern grep.
**Effect:** Removes one verification round-trip per review of text-edit seeds; small per run but every review pays it.

## 7. Fix the pipeline-progress denominator — "0 of 7" shown to a stage that had 2 completed
**Evidence:** Implementer's stage prompt (seq 70) opens with "Pipeline progress: **0 of 7** stages completed" although start and planner were both completed at that point; the run header later showed "6 of 7" post-closeout (also filed as fabro-a0e3/fabro-45bf).
**Change:** Engine stage-prompt renderer: compute progress from unique completed nodes.
**Effect:** Correct progress in every stage prompt and the web header — pure UX, zero risk.

**One deliberate non-recommendation:** the 20-minute tester timeout was not a problem here (gate ran 5.4 s via touched-crate scoping), so path-scoping the gate for prompt-only seeds (fabro-01b9) would save nothing on this run class; deprioritize it below items 1–4.
