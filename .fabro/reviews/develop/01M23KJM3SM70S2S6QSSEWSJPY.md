# Improve review — run 01M23KJM3SM70S2S6QSSEWSJPY

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (3.3 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-09 17:39+0000 by revisor `fabro_ask`

---

All evidence below is from this run's event history (seq numbers cited), worker log, and the stage timings/costs in the run conclusion. Run profile: 186.8 s wall, $0.181 total, planner $0.083 (46%), implementer $0.067 (37%), reviewer $0.031 (17%) — a one-paragraph prompt edit through a 7-stage graph.

## Recommendations, ordered by expected impact

**1. Kill the planner's tool-output firehose — it was 46% of run cost for a mechanical claim.**
- What happened: `sd ready --assignee fabro --limit 200` returned 191 issues / 25,780 bytes with `stdout_truncated: true` (seq 33) to pick one candidate that was already priority-1 at the top. `fabro_runs_list` returned 17 runs / 15,401 bytes (seq 35), 16 of them terminal with identical generic goal text; the planner's reasoning (seq 38) shows it burning tokens deducing "the only non-terminal run is this run itself". Planner: 50.4 s inference vs 3.1 s tool time.
- Change: `.fabro/workflows/develop/prompts/project-facts.md` command table + `.fabro/workflows/develop/prompts/planner.md` steps 1/4 — `sd ready --limit 10` (top-N view; the High seeds are what's ever claimed) and `fabro_runs_list` with `created_since ≈ 48h` plus an explicit self-run-id exclusion. (Already filed as seeds fabro-c3b4 and fabro-6b58 — landing them is the change.)
- Expected effect: ~41 KB less first-turn context on every develop run; planner input tokens (29.2k here) and reasoning drop materially. This is the single largest recurring cost in the loop.

**2. Planner briefs must never put the gate command in verification criteria — this run ran the identical gate twice.**
- What happened: the seed's verification bullet said "then `just qualitygate` per PROJECT_FACTS"; the implementer ran it (seq 111–113, 4.2 s, "no crates touched / format clean"), then the deterministic tester ran the byte-identical command on the unchanged tree (seq 128–129, 4.3 s, identical output). The implementer journaled the contradiction (implementer journal: "the seed's verification criterion required the implementer to run `just qualitygate` even though step 4 of the very prompt being edited forbids it").
- Change: `.fabro/workflows/develop/prompts/planner.md` step 7 (contradiction check) — add one rule: a verification criterion naming the PROJECT_FACTS gate command is rewritten to "gate green via the deterministic tester step" before the brief ships. The planner already knows the tester node exists; step 7 is where it should have caught this.
- Expected effect: eliminates the double gate entirely. Here it cost only ~4 s (markdown-only change), but on Rust seeds the same duplication is a cold-cache build measured at ~15 min worst case (the graph comment in `workflow.fabro` cites 8m46s build alone).

**3. Make the implementer's `sd show` re-fetch conditional — the spec was already verbatim in context.**
- What happened: step 1 of the implementer prompt says "Re-read the seed requirements from `sd show <current_seed_id>`"; the implementer spent a shell call + LLM turn fetching the full seed JSON (seq 84–87, 1,262 bytes) although `current_seed_brief` in its `## Context` already restated every acceptance criterion verbatim, including the full seed text.
- Change: `.fabro/workflows/develop/prompts/implementer.md` step 1 — "If the brief restates the acceptance criteria in full (as planner briefs do per step 6), skip the `sd show` re-fetch; fetch only when the brief is thin or marked verification-only." (Open seed fabro-4881 covers this — land it.)
- Expected effect: one fewer tool call and one fewer LLM turn per implementer pass (~7 s, ~$0.02 here); on every run that's ~5% of implementer cost for zero information gain.

**4. Stop per-stage metadata snapshots on command nodes — ~17 s of the 187 s wall was snapshot overhead.**
- What happened: eight metadata snapshots (init + one per stage boundary + finalize), each 1.8–2.5 s (seq 15, 69, 123, 133, 143, 169, 179, 188) ≈ 16.9 s total ≈ 9% of run wall — for a run whose command stages (tester/evidence/closeout) completed in 0.3–4.3 s each.
- Change: engine-side (open seeds fabro-c2ca "skip metadata snapshots on non-agent stages" and fabro-652d "batch checkpoint pushes at terminal boundaries") — land fabro-c2ca first, it's the cheaper cut.
- Expected effect: ~8–10 s per run (5%+ of wall on short runs, more on journal-heavy runs) with no information loss; non-agent stages produce no agent state worth snapshotting mid-run.

**5. Give the evidence capture its own context key / enough budget — the reviewer paid a blob detour for an 8.5 KB capture.**
- What happened: the evidence output (8,541 bytes) arrived as a blob ref (`/tmp/fabro/runtime/blobs/e1b0681f….json`) despite the reviewer's `preamble_inline_max_kb=16`, because the aggregate 24 KB graph budget was exceeded (reviewer prompt, seq 148). The reviewer correctly paged it with one `read_file` (seq 158–159, 12 ms — the prompt's blob instructions worked), but it also overwrote the tester's `command.output` key (seq 140), leaving key semantics dependent on node order.
- Change: `.fabro/workflows/develop/workflow.fabro` — take open seed fabro-9ef9 (evidence gets its own output key, e.g. `output.evidence`) and/or fabro-8d2c (`preamble_budget_kb` 24→32).
- Expected effect: no blob round-trip per review, and `command.output` stops meaning "whichever command node ran last" — removing the exact ambiguity that produced past verification-blocked cycles.

**6. Fix the warn that fires on every green run.**
- What happened: the worker log's only warning is `preamble_allow_keys entry absent from context node=implementer key=output.gate_known_bug_hits` — that key only exists after a gate-red bounce through gatebounce, so on a first-visit green path (this run, and the majority of runs) the lint warns unconditionally.
- Change: engine lint (open seed fabro-8275: downgrade by-design allow-key absence to info), or declare the key conditional in the implementer node's `preamble_allow_keys` documentation in `workflow.fabro`.
- Expected effect: warn-level logs become signal again; one less spurious line per run. Zero behavior change.

**7. Trim per-stage memory reload — AGENTS.md (24.6 KB) was loaded into all three agent stages, and no cache writes happened.**
- What happened: `agent.memory.loaded` shows `/workspace/fabro/AGENTS.md` (24,605 bytes, 5.7–6.4k tokens per stage) loaded at seq 23, 76, and 150; `cache_write_tokens: 0` on every stage while `cache_read_tokens` totaled 222k — the shared prefix is never written, so each stage re-pays the fixed preamble at full input price. Memory was ~22% of the planner's input tokens for a seed that touched one prompt file.
- Change: engine (open seeds fabro-9588 per-node memory scoping, fabro-944d shared prompt-cache prefix). Cheapest first step: scope memory per node — the planner/reviewer don't need the full repo agent guide that the implementer does.
- Expected effect: measurable input-token cut on all three stages every run; the planner stage benefits most since it's cost-dominant (see #1).

**What worked and needs no change:** the deterministic evidence/closeout nodes (0.4 s / 0.3 s), the reviewer's low reasoning effort + inline PASS/FAIL report (approved in 22.5 s, $0.031, one tool call), the gatebounce design (unexercised but correctly off the green path), and the planner's stale-basis check (seq 45–56) — it spent three cheap greps verifying the basis and correctly resolved the "is the rule already scoped?" doubt before claiming.
