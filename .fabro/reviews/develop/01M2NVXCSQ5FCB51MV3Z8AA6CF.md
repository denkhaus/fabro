# Improve review — run 01M2NVXCSQ5FCB51MV3Z8AA6CF

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (23.6 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-16 20:33+0000 by revisor `fabro_ask`

---

All facts below are from this run's events, stage journals, worker logs, and the tracker file `.seeds/issues.jsonl` (checked for existing seed coverage per candidate change).

**Run baseline (from run events/conclusion):** 23.5 min wall, $0.807 total, zero stage retries. Implementer dominated: 888 s wall (65%) and $0.617 (76% of cost, 41 tool calls incl. 2 `edit_file` errors). Planner: 242 s / $0.147. Tester gate: 216 s. Reviewer: 20.5 s / $0.043. Seed fabro-79d8 claimed → implemented (+400 lines, one file) → gate green first try → approved → closed; PR #201.

## Recommendations, ordered by expected impact

**1. Closeout must re-file unmet user-action arms before closing the seed.**
Evidence: fabro-79d8 was closed at 20:06:23 while its demand 1 (deploy the #199 fix via `just image-release` + tofu apply) is unmet — the reviewer's own journal says "the stall will recur until deployed." The deploy reminder now survives only inside a closed run's journal, which the loop's own prompts call a place "nobody re-reads." The run reported success while the live mirtuell outage persists.
Change: in `.fabro/workflows/develop/scripts/closeout.nu` (or the planner's pre-approval step in `prompts/planner.md`), when a brief carries an EXEMPTION bullet, file that arm as a new open OPS seed before `sd close`.
Expected effect: the actual outage fix (deploy) stays tracker-visible instead of dying with the closed seed.
**New-seed justification:** fabro-ae74 covers structured arms *lists* at intake; no existing seed covers re-filing *unmet* arms at closure — and the planner's own painpoint this run asked for exactly this split.

**2. Fix verify.nu's blindness to inline `#[cfg(test)]` modules — seed fabro-2f70 (open).**
Evidence: the implementer's journal this run: "Verify dispatcher classed the diff as code-touched only (tests live in the same file as the code); I additionally ran the targeted scheduler suite myself" — a manual 25-test run absorbed into the dominant stage's 547 s of tool time.
Change: in `scripts/verify.nu`, classify a touched file whose diff edits inside `mod tests`/`#[cfg(test)]` as test-file-touched, so the mechanical verify runs the full crate suite itself.
Expected effect: the $0.617/888 s stage stops paying manual compensation, and runs where the implementer *doesn't* compensate keep pre-gate test coverage.

**3. Top-N `sd ready` view — seed fabro-c3b4 (open).**
Evidence: the planner's first call (event seq 30) pulled 200 issues / 28,448 bytes to pick one candidate; planner conversation tokens reached 28 k of its 38.7 k input.
Change: command table in `.fabro/workflows/develop/prompts/planner.md` — `sd ready --limit 10` for candidate selection (full listing only on demand).
Expected effect: ~28 KB less planner context per run → faster/cheaper claims, less tie-break distraction.

**4. Raise the reviewer's inline evidence ceiling — seed fabro-cf3e (open, target already amended to ≥40 KB).**
Evidence: this run's evidence capture was 23,331 bytes and was demoted to a blob ref by the reviewer's stale 16 KB `preamble_inline_max_kb` — despite the graph's 48 KB budget raised explicitly (fabro-1e9f) to make captures "zero blob detours." The reviewer spent its only tool call paging the blob.
Change: `.fabro/workflows/develop/workflow.fabro`, reviewer node: `preamble_inline_max_kb=16` → 40.
Expected effect: captures ≤40 KB arrive inline; one fewer tool round trip per review and the unread-blob rejection class disappears.

**5. Make the journal JSON payload retry-proof — seed fabro-270c (open).**
Evidence: the planner's first final answer (seq 78) emitted malformed routing JSON ("key must be a string at line 8 column 363" — a stray array after the journal object), costing a validation round trip fixed only at seq 83 (~10 s). The same malformed-JSON class hit the PR postlude at 20:06:38 ("trailing comma", retried non-strict) — covered by open fabro-41b1.
Change: in `prompts/planner.md` (and implementer.md), render the journal object as a literal fill-in template with both keys pre-nested, not a prose description.
Expected effect: no schema-validation retry per pass; removes the risk of burning `output_retries=2` into a node failure.

**6. Symbol search over area-prefix path guessing — seed fabro-1a35 (open).**
Evidence: the planner's basis probes (seq 43–57) guessed `lib/components/fabro-workflow/src/automation_scheduler.rs` (nonexistent), then `fabro-automation/src/`, before an `rg -l` found the real target `lib/apps/fabro-server/src/server/automation_scheduler.rs` — three probe rounds and two extra LLM turns.
Change: one line in `prompts/planner.md` step 3: resolve seed-named paths by `rg -l <symbol>` first, never by crate-area prefix.
Expected effect: 2–3 fewer tool calls and ~20–30 s less planner latency per claim; fewer wrong-path briefs.

**7. First-token stall watchdog for stage LLM calls — new seed needed.**
Evidence: one planner LLM call waited 163 s for its first token (llm.started seq 70 at 19:44:17 → first_output seq 71 at 19:47:00) — 67% of the entire 242 s planner stage; no error, so nothing retried.
Change: engine-side per-call TTFT timeout with transparent retry/fallback in the stage LLM client (near `lib/components/fabro-workflow/src/model_fallback.rs`).
Expected effect: bounds planner latency against provider stalls; here it would have cut ~2.7 min from a 4-minute stage.
**New-seed justification:** fabro-83af covers stage-level retry with backoff; no existing seed covers intra-call first-token stalls (a hung call never fails, so no retry fires).

**8. Demote expected-missing memory-file errors — seed fabro-a701 (open).**
Evidence: worker logs show 6 ERROR lines this run — every agent session init failed reading `/workspace/fabro/.codex/instructions.md`, which doesn't exist in this repo. (Same logs: 18 "tool result error flag unsupported" warnings matching open fabro-b09c.)
Change: log absent *optional* memory files at info in agent session init.
Expected effect: warn/error views stop filling with non-failures, making real signal (like the JSON retries above) easier to spot.

**Not recommended despite temptation:** the tester/evidence/closeout nodes were near-optimal this run (216 s / 0.3 s / 0.4 s, zero retries) — no graph restructuring there is warranted by this run's data.
