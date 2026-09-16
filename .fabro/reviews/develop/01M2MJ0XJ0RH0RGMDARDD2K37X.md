# Improve review — run 01M2MJ0XJ0RH0RGMDARDD2K37X

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (13.2 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-16 07:48+0000 by revisor `fabro_ask`

---

## Run grounding (from run events, logs, and stage timings)

Seed **fabro-02c4** ("Closeout closure discipline"), first-pass approve: planner → implementer → tester → evidence → reviewer → closeout, zero retries. Wall **755.6 s**, cost **$0.431**. Split: implementer **592.6 s inference / $0.294 (69% of cost, 79% of wall)** vs 2.9 s tool time (24 shell calls, 0 errors); planner 90.7 s / $0.093; reviewer 47.1 s / $0.045; tester 3.6 s; evidence 0.3 s; closeout 0.35 s.

## Recommendations, ordered by expected impact

1. **Slim the implementer's step 4 — it is the run.** Evidence: implementer prompt carries a ~13 KB essay in step 4 (gate ban, crate classes, feature-flag rules, transform rules), resent across ~20 LLM turns; 592.6 s of the 755.6 s run was inference between sub-second tool calls (stage timing, run events seq 84–100+). Change: `.fabro/workflows/develop/prompts/implementer.md` step 4 → cap to the operative rule (one mechanical verify call, crate-class table), move run-id citations to a footnote block. **Seed: fabro-7b2a** ("cap step 4 operative rule, move run citations to footnotes"). Effect: shorter per-turn processing on the stage that owns ~80% of wall and ~70% of cost; even −20% ≈ 2 min/run.

2. **Stop flooding the planner with the full backlog.** Evidence: `sd ready --assignee fabro --limit 200` returned 200 seeds / **27,766 bytes, `stdout_truncated: true`** (run events seq 30) — to pick ONE seed. Change: `.fabro/workflows/develop/prompts/planner.md` + PROJECT_FACTS sd table → priority-sorted top-N (≤10) invocation. **Seed: fabro-66bc** (top-N priority-sorted `sd ready`; adjacent batching: fabro-55a7/fabro-2be2 — the planner used 6 sequential shell round-trips at seq 29/35/43/49/55/61, each costing a 6–17 s LLM gap). Effect: ~28 KB less context and 1–2 fewer LLM turns per planning pass.

3. **Deliver the evidence capture inline — the blob detour fired again.** Evidence: capture was **23,761 bytes vs the reviewer's `preamble_inline_max_kb=16`** → blob-ref marker, one wasted `read_file` round-trip before judging (reviewer stage tools: 1 call; run events seq 241 prompt). Change: `reviewer` node in `.fabro/workflows/develop/workflow.fabro` → `preamble_inline_max_kb=32`. **Seed: fabro-cf3e** (raise 16→32; graph-budget counterpart fabro-8d2c). Effect: review starts from the full diff inline; removes the unread-blob rejection class entirely.

4. **Make the new closeout "park" visible at the terminal state.** Evidence: this run added the park branch (`closeout.nu` parks with **exit 0**, seed stays open); the implementer's journal flags "if the engine treats a green non-closing closeout specially, that interaction is untested" — today a parked run exits `succeeded/natural`, indistinguishable from a real close in the terminal event and Slack payload. Change: `.fabro/workflows/develop/scripts/closeout.nu` park marker + surface "seed parked (open)" in the terminal event/notification (or route a soft exit). **New-seed justification:** the park exit shape was introduced by fabro-02c4 in THIS run; fabro-5b0a/fabro-b1d3 target planner fail-open checks and bookkeeping-only runs, not the closeout park. Effect: the user and the next planner see the hold without grepping the journal.

5. **Exclude `.mulch` from the demand-visibility patch.** Evidence: reviewer journal — "demand-visible token matching does not exclude `./.mulch/` … mulch records quote seed descriptions verbatim"; confirmed in the run diff: `seed-demand-visible` excludes only `:(exclude).seeds` and `:(exclude).fabro/journal` while `.mulch/expertise/workflows.jsonl` gained mx-65fbb4 quoting the seed title. Change: one more pathspec in `seed-demand-visible` in `.fabro/workflows/develop/scripts/closeout.nu`. **New-seed justification:** no open seed covers .mulch exclusion in closeout.nu — fabro-8a60 is qualitygate-side, fabro-6db3 is evidence.nu churn classification. Effect: closes the false-close loophole for churn-only runs (the exact fabro-9967 class this seed exists to prevent).

6. **Backfill PR state for terminal-but-unmerged runs.** Evidence: the in-flight check had to judge PR 161 (run 01M2MGAQ4, status `succeeded`) with `pull_request.state: null` — the planner noted "state null — not open" as a guess (journal; run events seq 39). Change: engine `fabro_runs_list` projection (server-side, no prompt change). **Seed: fabro-4bb7.** Effect: the claim-race guard stops running on null-state guesses; prevents double-picking a seed whose PR is actually still open.

7. **Drop the strict-JSON first attempt for PR bodies.** Evidence: run log 07:44:12 — "PR content structured generation failed; retrying once without strict JSON output … model did not return a JSON document" (PR-body model `zai:glm-4.7`). Change: PR postlude in the engine's pull_request pipeline → non-strict parse first. **Seed: fabro-41b1.** Effect: one fewer wasted LLM call per run and failures stop being masked by the retry.

8. **Fix the pipeline-progress counter in stage prompts.** Evidence: reviewer prompt header said "Pipeline progress: **2 of 7**" when 5 stages (start…evidence) were completed; implementer header said "0 of 7" after two (run events seq 77/241 vs checkpoint completed_nodes). Change: engine prompt-header computation → unique completed nodes. **Seed: fabro-45bf.** Effect: correct stage framing for downstream agents and the UI.

9. **Quiet log noise that buries real signals.** Evidence: run logs carry **6× ERROR** for missing `/workspace/fabro/.codex/instructions.md` (2 per agent session init) plus the by-design `preamble_allow_keys … output.gate_known_bug_hits absent` warn. Change: **Seed fabro-8275** for the allow-key warn→info; for the `.codex` probe, downgrade the optional-file miss to debug. **New-seed justification (codex part):** no open seed covers the optional memory-file probe noise. Effect: warn/error views become actionable again.

**Not recommended:** restructuring tester→evidence→reviewer sequencing or the gate bounce path — this run exercised none of the cycle machinery (first-pass green, tester 3.6 s on a config-only diff), so there is no evidence of benefit; the deadlock guards and cheapest-first verification behaved exactly as designed.
