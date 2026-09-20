# Improve review — run 01M23N9E3R1GKHRRA4MPB3GY2A

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (2.7 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-09 18:09+0000 by revisor `fabro_ask`

---

All evidence below is from this run's events (seq numbers), the run log, and the stage timings/billing in the run projection. Context: seed `fabro-f7cf` (a one-line edit to `planner.md` step 7), 2m32s wall, $0.166 LLM total, first-pass approval, zero gate bounces or retries. The graph itself needed no rescue this run — every finding below is about cost, latency, or signal quality inside a healthy pass.

## Recommendations, by expected impact

**1. Bound the in-flight check: `fabro_runs_list` with `created_since` + self-exclusion**
- What happened: planner's guard call (seq 38/41) returned **18 runs / 16,310 bytes — 17 of them terminal** with identical generic goal text. The planner then had to reason "the only non-terminal run is this run itself" (seq 44) to conclude fabro-f7cf was claimable.
- Change: `.fabro/workflows/develop/prompts/planner.md`, step 4 (IN-FLIGHT PR CHECK) — one sentence: pass `created_since ≈ 48h` (the tool already supports it, per its own description) and exclude this run's own id from the in-flight set. This is open seed fabro-6b58, grounded in the sibling run 01M23HWFFSB9NK78PGSCZWE6MA and re-proven here.
- Effect: ~16 KB → ≤2 KB of planner context every run; removes the self-run confusion class entirely.

**2. Top-N `sd ready` view instead of the 194-row firehose**
- What happened: `sd ready --assign 200` (seq 31–33) dumped **26,246 bytes / 194 seeds** into planner context; only the first ~20 High rows were ever used (it picked row 1). This is the single largest context item in the run and the planner is the costliest stage ($0.069 / 40.7s / 29.5k input tokens — 42% of run cost for a one-line edit).
- Change: `.fabro/workflows/develop/prompts/project-facts.md` (sd command table) + `planner.md` step 1 — list top ~20 by priority with an explicit "run `sd ready` again with a wider window only when the top rows are all blocked/in-flight" (open seed fabro-c3b4).
- Effect: ~20k fewer planner input tokens (~$0.03 and ~10–15s per run); also shrinks the mispick surface.

**3. Share the prompt-cache prefix across stages (engine)**
- What happened: **`cache_write_tokens: 0` in every stage** while 212,928 tokens were cache-*read*. The reviewer's first LLM call (seq 156) got **0 cache hits** and paid full input (14.8k tokens, $0.022 of its $0.031) despite ~24.6 KB of AGENTS.md memory plus the ~5 KB PROJECT_FACTS block being byte-identical to what planner/implementer just sent.
- Change: engine-side prompt ordering so the stable prefix (system + memory + PROJECT_FACTS) precedes per-stage content — open seed fabro-944d (`lib/components/fabro-workflow`).
- Effect: reviewer and implementer first calls stop paying uncached input; realistic 20–30% cut of the $0.166 per run, larger on Rust seeds with longer stage transcripts.

**4. Name the shell bypass in the fs_hide denial (and skip the doomed `edit_file` entirely)**
- What happened: despite the brief explicitly warning "`.fabro/` is fs_hide — edit via shell," the implementer's first write attempt was `edit_file` on `planner.md` (seq 93) → denied (seq 94), costing a wasted LLM round-trip (~8s, $0.007) before the python3-heredoc fallback (seq 98) worked.
- Change: (a) engine: make the fs_hide denial message say "use the shell (sed/python3) for this path" — open seed fabro-d02d; (b) `.fabro/workflows/develop/prompts/implementer.md` envelope section: one line "when the brief targets an fs_hide path, do not attempt file tools at all."
- Effect: eliminates one wasted turn on **every loop-asset seed** (this repo's most common seed type — 3 of the 4 newest seeds target prompts).

**5. Batch checkpoint snapshots/pushes at terminal boundaries**
- What happened: eight serialized metadata snapshots (init 2.7s, then 1.9–2.4s per stage boundary + finalize 2.0s = **~17.2s, ~11% of the 2m32s wall**), each also committing to the meta branch with growing payloads (117 KB → 374 KB) and pushing the run branch 7 times for a PR that squashes anyway.
- Change: engine, open seed fabro-652d — snapshot/push at terminal and soft-exit boundaries only.
- Effect: ~12–15s off every run's wall time; identical recovery semantics for the failure cases the checkpoints exist for.

**6. Stop the unconditional `sd show` re-fetch and the 80-line `sed` probe**
- What happened: implementer re-fetched the seed via `sd show` (seq 78/82) — byte-identical to the brief already in its `## Current context` — and read `sed -n 1,80p planner.md` (**13.7 KB**, seq 79–81) when the brief pinned line 43 exactly.
- Change: `.fabro/workflows/develop/prompts/implementer.md` step 1 — make the `sd show` re-fetch conditional on the brief being thin (open seed fabro-4881), and add "when the brief pins a line anchor, read `sed -n '<n>p'` only" (open seed fabro-645d).
- Effect: 1–2 fewer tool calls and ~15 KB less implementer context per run.

**7. Downgrade the by-design `preamble_allow_keys` warn**
- What happened: the run log's only WARN: `preamble_allow_keys entry absent … node=implementer key=output.gate_known_bug_hits` (18:02:06). That key only exists after a gate-red bounce; on green runs like this one its absence is by design, so the warn fires as pure noise once per run.
- Change: engine lint severity — open seed fabro-8275 (warn → info, or suppress when the producing node `gatebounce` never ran).
- Effect: warnings become actionable; a real allow-key drift stops hiding behind a known-benign line.

**8. Port the "workaround is a painpoint" clause to the implementer journal contract**
- What happened: the implementer journaled the fs_hide shell-write as an *observation* ending "no denials wasted" — but seq 94 shows a denial was exactly what happened. The friction signal never reached the painpoint channel the revisor scans; the reviewer prompt has this clause, `implementer.md` doesn't (open seed fabro-21c0).
- Change: `.fabro/workflows/develop/prompts/implementer.md`, Journal section — copy the reviewer's "a workaround you performed is a painpoint, not an observation" line.
- Effect: engine-side friction like recommendation 4 gets reported accurately instead of being misclassified as clean.

## Not inspected
PR #105's final merge state (`pull_request.state` is null in the summary and no merge event appears in the inspected range — only creation at seq 186), so I can't assess the postlude/auto-merge cost. Everything above is from run events seq 1–200, the worker log, and the stage billing records.
