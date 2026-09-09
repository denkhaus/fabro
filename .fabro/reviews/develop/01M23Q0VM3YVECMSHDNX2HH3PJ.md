# Improve review — run 01M23Q0VM3YVECMSHDNX2HH3PJ

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (12.3 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-09 18:48+0000 by revisor `fabro_ask`

---

All evidence below is from this run's events (seq numbers), stage timings/cost from the run conclusion, and the worker log. Run totals for scale: 720s wall, $0.502 LLM cost; implementer alone was 593s (82% of wall) and $0.383 (76% of cost). The run itself was clean — first-pass gate green, first-pass approval, zero retries — so the levers are cost/latency hygiene and two observed near-misses.

## 1. Stop prescribing the full crate suite for seeds that only ADD tests (prompting — biggest cost lever)
- **Change:** In `.fabro/workflows/develop/prompts/implementer.md` step 4, scope the "test-file-touched crate" rule: full `cargo nextest run -p <crate>` only when the seed *edits existing tests*; for purely added tests, run the new tests by name filter. Mirror this in `.fabro/workflows/develop/prompts/planner.md` step 6 so briefs stop prescribing the full suite (this run's brief criterion 5 literally named `cargo nextest run -p fabro-workflow`).
- **Evidence:** The implementer ran the focused tests (seq 230, cold build ~4.7 min), a rerun+clippy (seq 247, ~102s), then said "Since I touched test files in `fabro-workflow`, run the full crate suite" (seq 252) and ran all 1543 tests (seq 253, ~56s) — to validate 2 added unit tests the deterministic tester re-ran 23s later ("tests green" in the tester output).
- **Expected effect:** ~1 min wall plus one LLM turn saved on every test-adding seed; the gate's coverage is unchanged.

## 2. Cap the planner's `sd ready` firehose (tool usage)
- **Change:** In the planner command table (`prompts/planner.md` PROJECT_FACTS), change the entry call to a top-N view (e.g. `--limit 10`) with fallback to the full listing only when no candidate is claimable (already filed as seed fabro-c3b4; this run is fresh evidence for it).
- **Evidence:** `sd ready --assignee fabro --limit 200` returned **193 issues / 26,096 bytes, stdout truncated** (seq 33–34) — the planner needed only the top of the list to pick fabro-6a5a. That ~26KB rode into a planner pass that cost $0.085 (17% of run) with 41s inference.
- **Expected effect:** ~20KB less context and faster deliberation per planning pass; removes truncation-induced blind spots in the candidate list.

## 3. Make the postlude respect a run's own fix (graph/engine — UX)
- **Change:** In the engine's postlude settings resolution (`lib/components/fabro-workflow/src/pipeline/publish.rs` / run-spec handling), either re-resolve `[run.pull_request]` from the run branch tip when the run's own diff edits its `workflow.toml`, or append a "takes effect next run" notice to the PR body.
- **Evidence:** The run spec snapshot at creation (event seq 1) still carries `auto_merge = true` — the very value this seed set to `false`. So PR #107's postlude ran under the pre-fix settings; the two recurring failures the seed exists to kill persisted on the fixing run itself (postlude activity until 18:45:34, ~1m46s after completion).
- **Expected effect:** No misleading failure/warn on the run that shipped the fix; users stop seeing the "fixed" bug one last time.

## 4. Make the implementer's seed re-fetch conditional (prompting/tool usage — free win)
- **Change:** In `prompts/implementer.md` step 1, skip the mandatory `sd show` when the brief is complete and carries no `review_feedback` (matches open seeds fabro-4881/fabro-a67f).
- **Evidence:** The planner wrote the corrected seed body at 18:32:10 (seq 57) and embedded it in the brief; the implementer re-fetched the byte-identical JSON at 18:32:32 (seq 84/88) — one wasted shell call plus one LLM turn, every run.
- **Expected effect:** One fewer call and LLM round-trip per implementer pass (~5–10s, small token saving, zero risk).

## 5. Downgrade the `output.gate_known_bug_hits` preamble warn (error handling hygiene)
- **Change:** In the engine's preamble-allow-keys lint (`lib/components/fabro-workflow/src/handler/…`, preamble rendering), expect that key only when the last tester outcome was red — or downgrade to info (seed fabro-8275).
- **Evidence:** The worker log's **only** warn line in this entire run is `preamble_allow_keys entry absent … node=implementer key=output.gate_known_bug_hits` — it fires on every green-path run because the key only exists after a gate-red bounce.
- **Expected effect:** Warn stream carries signal only; real degradations stop competing with structural noise.

## 6. Add probe-command discipline to the planner (error handling near-miss)
- **Change:** One line in `prompts/planner.md` (step 3): never append `2>/dev/null` to probe commands; always quote sed ranges (`sed -n '40,70p'`).
- **Evidence:** The planner's basis probe (seq 45/51) began with a malformed `sed -n 40..70p 2>/dev/null;` — silently empty — and was saved only by the correctly-quoted second sed in the same command.
- **Expected effect:** Eliminates the silent-empty-probe failure class at zero cost.

**What I could not verify:** whether the auto-merge GraphQL warn actually re-fired on PR #107 (server-side postlude events aren't in the worker log I can read); rec 3 rests on the run-spec snapshot at seq 1, which is unambiguous.
