# Improve review — run 01M5KCXWDYM96AVJM95S51A1Y3

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (11m32s, revisor pass — reason and cost in run detail)
- generated: 2026-10-23 06:10+0200 by revisor `fabro_ask`

---

All evidence is in hand. Here are the recommendations, grounded in this run's actual behavior, ordered by expected impact.

**Run facts (from run events/conclusion):** wall 11m32s, cost $0.390, zero retries, one green pass for a +2/−2 prompt edit (`.fabro/workflows/revisor/prompts/analyze.md`). Planner = $0.203/307s (52% of cost), implementer = $0.134/285s, reviewer = $0.053/44s; all command stages combined ~21s/$0. Tracker dedup searches run: `fabro-30a5`, `title`, `in-flight check planner`, `prompt-lint output`, `closeout deferred sweep`, `precondition brief`, `run_search`, `preflight legend` (plus the 65-row ready list visible in the planner transcript).

1. **fabro-c3b4 — top-N ready view in the planner.** `seeds ready --assignee loop --limit 200` dumped 65 rows / 9,491 bytes into the planner (planner transcript, seq 92–95); the claim only needed the top candidate. Change `.fabro/workflows/loop/prompts/planner.md` step 1 (and `planner-preflight.nu` candidate emission) to a top-≈5 view. Effect: the most expensive stage reads ~9KB less and skips turns reasoning past 60 irrelevant rows.

2. **fabro-a827 — diff-scope the loop gate's prompt-lint.** Tester output was 8,704 bytes for a 2-line edit, all 37 warnings about untouched files (tester stage output). Change `.fabro/workflows/loop/scripts/loop-gate.nu` to lint only diff-touched files (full-repo lint stays in develop's gate). Effect: tester output ~8.7KB → <1KB, which also shrinks the evidence capture and reviewer preamble downstream.

3. **New item — closeout files a `deferred-action: none` observation as a seed.** Closeout output "✓ Created fabro-30a5", whose tracker title is literally "Deferred follow-up from fabro-1923: none — the open dependency fabro-4653…" — filed from the implementer's observation that explicitly began `deferred-action: none`, duplicating already-open fabro-4653. *Justification: no existing item covers the loop closeout mis-parsing the none-marker (searched "closeout deferred sweep" → 0; fabro-db25 covers only the revisor closeout's materiality bar).* Change `.fabro/workflows/develop/scripts/closeout.nu` deferred-sweep arm to skip `deferred-action: none` observations, and close fabro-30a5 as superseded (dup of fabro-4653). Effect: no junk row polluting every future `seeds ready`/preflight scan.

4. **fabro-ab38 — sweep satisfied sibling seeds after merge.** Open seed fabro-9dd6 ("analyze.md's verbatim fabro_ask question still says 'known seed id fro…") describes exactly what this run landed; it stayed open through the claim and will produce a redundant future run. Change: land fabro-ab38's sweep (close fabro-9dd6 as `superseded: fix landed in <this run's sha>` once merged). Effect: prevents an entire wasted loop run on already-satisfied work.

5. **fabro-17fb — stop the reviewer reading the lint output twice.** Reviewer preamble carries the tester stage section AND evidence's "recorded checks" re-prints the same prompt-lint/lint-nu output verbatim (evidence output 12,580 bytes vs tester's 8,704, largely overlapping). Change `.fabro/workflows/develop/scripts/evidence.nu` (--lane loop) to collapse duplicated recorded checks to a pointer (or drop tester's `command.output` from the reviewer node's `x.preamble_allow_keys` in `.fabro/workflows/loop/workflow.fabro`). Effect: ~10KB less reviewer input per review.

6. **fabro-eaa0 — require English implementer responses.** The implementer's entire final response — which became `implementation_summary` and the stage output — is in German (implementer stage transcript). Change: one language line in `.fabro/workflows/loop/prompts/implementer.md`. Effect: summaries readable by the reviewer stage, PR, and human consumers.

7. **New item — briefs must adjudicate named preconditions.** Implementer journal painpoint: seed fabro-1923's action was conditional on the phase-3 rewrites, fabro-4653 was still open, and "the precondition judgment fell to the implementer" — the planner had adjudicated it only as a journal observation, not a brief bullet. *Justification: no existing item (searched "precondition brief" → 0; nothing in the ready list covers it).* Change `.fabro/workflows/loop/prompts/planner.md` brief-writing step: one met/waived bullet per seed id the claimed body names as precondition. Effect: claim-eligibility decisions stop being re-derived downstream.

8. **fabro-b529 — run-title cap retry waste.** `fabro_run_set_title`: 3 calls, 2 rejections against the 100-char cap before the 98-char title stuck (planner agent tool stats). Change the RUN TITLE paragraph in `.fabro/workflows/loop/prompts/planner.md` to prescribe a ≤90-char template. Effect: one title call instead of three; fewer planner turns.

9. **fabro-1cde — `fabro_run_search` unknown-status error.** The planner's in-flight check passed `status: ["in_progress",…]` — a seed status, not a run status — and got `unknown run status 'in_progress'` (1 of 2 calls errored; run events seq 96–98). Change: the tool error names valid RunStatusKind values (fabro-1cde), and meanwhile planner.md step 5 lists the exact non-terminal run statuses. Effect: no burned retry turn on the first planner visit of every run.

10. **fabro-b769 — long-stage heartbeat.** Planner (307s) + implementer (285s) = 9.9 of 11.5 minutes fully silent; Slack notifications fire only on `run.completed`/`run.failed` (run settings). Change: heartbeat notification (hook or notification config) when a stage exceeds ~3m. Effect: operators see liveness during the two 5-minute agent stages instead of a 12-minute blackout.

11. **fabro-801e — retire stale prompt-lint date pins.** 20 of the 37 warnings are date pins older than 45 days (2026-04-14 → 09-08 across conductor/develop/loop/revisor files), printed in both tester and evidence every run. Change: re-validate or retire the 20 pinned sites prompt-lint names. Effect: removes the root of the recurring warning noise (complements #2).

12. **fabro-7808 — document grep exit-1 as expected no-match.** The implementer's pre-change probe exited 1 (is_error) purely because its final chained grep ("seeds CLI still named in develop prompts") matched nothing, while all useful output printed fine (implementer transcript, call_2f0a…). Change: add the expected-no-match line to the probe section of `.fabro/workflows/loop/prompts/implementer.md`. Effect: probe failures stop being misread as errors.

**What I could not inspect:** the two rejected title strings (only error counts are retained) and the PR-creation transcript (glm-4.7, 93s post-run) — both minor; nothing above depends on them.
