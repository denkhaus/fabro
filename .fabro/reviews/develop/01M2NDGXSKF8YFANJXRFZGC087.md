# Improve review — run 01M2NDGXSKF8YFANJXRFZGC087

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (6.6 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-16 15:42+0000 by revisor `fabro_ask`

---

All evidence below is from this run's event stream, worker log, stage journals, and stage timings (run `01M2NDGXSKF8YFANJXRFZGC087`, seed `fabro-4b76`, PR #191, 6m09s active, $0.331 total: planner $0.0995 / implementer $0.181 / reviewer $0.0501). Recommendations ordered by expected impact.

## 1. Fix the brief's `' / '` collapse before anything else
**What happened:** The planner emitted a newline-bulleted brief (event seq 73), but both the implementer prompt (seq 80) and reviewer prompt (seq 220) rendered `current_seed_brief` as one pipe-table row with ` / ` separators — "Acceptance criteria: / - … / - …". Every downstream PASS/FAIL check parsed criteria mush.
**Change:** Engine preamble renderer (`lib/`, per the graph's own comment) — emit multi-line values as bullets below the table instead of collapsing.
**Expected effect:** Checkable bullets actually reach the implementer and reviewer; removes a whole misparse class from every run.
**Seed:** `fabro-260c` (open; this run is a fresh recurrence — `fabro-9e49` covers only planner-side emission).

## 2. Wire the new fixture battery into the gate
**What happened:** The reviewer's journal painpoint (checkpoint seq 248): the gate lint-checks nu scripts but never *executes* `.fabro/scripts/dup-run-check-fixtures.nu`, so the seed's "battery exits 0" criterion "is only pinned by the implementer's word and my one re-run" — the reviewer spent one of its two tool calls re-running it (7/7 PASS).
**Change:** Add a `verify scripts` recipe (or a qualitygate substep) in `justfile` / `scripts/qualitygate.nu` that runs checked-in fixture batteries under `.fabro/scripts/`.
**Expected effect:** Regressions in dup-run-check semantics surface deterministically at the tester node; reviewers stop re-proving executable criteria by hand.
**New-seed justification:** No open seed covers executing fixture batteries in the gate — `fabro-50f8` (null-path exercises) and `fabro-8a60` (diff-presence check) are adjacent authoring/diff checks, not gate execution.

## 3. Raise the reviewer's inline cap so the evidence capture stops blob-ref'ing
**What happened:** The 19.9 KB evidence capture was demoted to a blob ref in the reviewer preamble ("Output (19.9 KB; full value: /tmp/…)") even though the graph budget is 48 KB — because the reviewer node's `preamble_inline_max_kb=16`. Reviewer journal: "required paging via the materialized path."
**Change:** `.fabro/workflows/develop/workflow.fabro`, reviewer node: `preamble_inline_max_kb` 16 → 32.
**Expected effect:** Captures up to ~32 KB render inline; no blob-read tool round-trip per review (~1 tool call + paging latency saved every green run).
**Seed:** `fabro-cf3e` (open; `fabro-9837` is the complementary marker-size fix).

## 4. Stop feeding the planner a 200-seed firehose
**What happened:** First planner shell call returned `sd ready --limit 200` output truncated at 28 KB / "200 ready issue(s)" (seq 30); the planner then made 6 sequential shell calls (seq 29–63). Planner wall 80.7s (75.9s inference, 4.8s tools) — 30% of run cost for a deterministic top-pick.
**Change:** `.fabro/workflows/develop/prompts/planner.md` step 1 + the PROJECT_FACTS sd table — use a top-N `sd ready` view (and batch recon per `fabro-55a7`).
**Expected effect:** Planner input tokens and turn count drop materially on every run; the claim decision stays identical (priority order is already in the listing).
**Seed:** `fabro-c3b4` (open; batching complement `fabro-55a7`).

## 5. Bound the in-flight check and make seed ids mechanical
**What happened:** The planner's `fabro_runs_list` (seq 38–39) took ~4.2s and returned 26 runs — every goal the identical generic string, no seed id — forcing the model to reason "only this run itself is non-terminal… self, exclude" (seq 42) with no `created_since` bound.
**Change:** Planner prompt step 4 + tool call: pass `created_since`, exclude self mechanically; engine-side, expose `current_seed_id` in the runs-list projection.
**Expected effect:** The double-pick/claim-race guard stops depending on model arithmetic over a 26-row wall; shorter planner pass, auditable skips.
**Seed:** `fabro-6b58` (bound + self-exclusion), with `fabro-9372` as the engine-side half.

## 6. Make PR-body generation non-strict
**What happened:** Worker log 15:38:17: `PR content structured generation failed; retrying once without strict JSON output error=LLM generation failed: the model did not return a JSON document` — the PR #191 postlude burned a failed strict call + retry at the worst moment (run terminal).
**Change:** PR postlude (`lib/components/fabro-workflow`, `pipeline::pull_request` path): default to the non-strict path for PR-body prose.
**Expected effect:** Removes a guaranteed retry latency/noise bubble on PR creation; the retry already proves the fallback works.
**Seed:** `fabro-41b1` (open, exact match).

## 7. Carry tool errors through the provider protocol
**What happened:** 15 worker-log WARNs: "this provider protocol does not support the tool result error flag" (zai/glm-5.3). Consequence visible in-stream: the planner's expected-no-match grep exited 1 (seq 51, `is_error: true`), the flag couldn't be conveyed, and the very next probe drifted into `2>/dev/null` (seq 56) — the exact anti-pattern `fabro-b6f9` forbids, adopted because stderr state was invisible.
**Change:** Engine agent protocol (`pebble_coding_agent::types` path): convey tool-result error state to the model (or strip the flag at a protocol boundary instead of warning 15×).
**Expected effect:** Models stop re-deriving error state from stdout sniffing and stop suppressing stderr in probes; fewer wasted probe turns.
**Seed:** `fabro-b09c` (open; the `2>/dev/null` drift is `fabro-b6f9`'s territory).

## 8. Classify seed-spec-named paths as seed-work in evidence
**What happened:** The evidence header read `seed-work=0 files +0/-0 | loop-churn=6 files +94/-4` for a seed whose spec names exactly those files (`implementer.md`, `analyze.md`, `facts.md`, the new battery). Review still worked only because the churn-only branch makes the loop diff the scope — on any mixed run this same misclassification becomes the `fabro-d76c` false-positive anomaly section.
**Change:** `.fabro/workflows/develop/scripts/evidence.nu`: diff-files intersecting the seed spec's named paths count as seed-work.
**Expected effect:** Correct review-scope labeling; kills the anomaly-section false positive on every loop-asset seed.
**Seed:** `fabro-93a7` (open; `fabro-d76c` is the adjacent false-positive report).

**Lower-tier, still grounded:** 6 ERROR-level worker-log lines for a missing optional `/workspace/fabro/.codex/instructions.md` (2 per agent session × 3 sessions) — log at info per open seed `fabro-a701`; and the implementer's mid-flight nu `rm -rf`-cwd lesson (`mx-95c645`, recorded this run) is repeat-avoidance evidence for open `fabro-d19e` (nushell skill).

What I could not inspect: the PR #191 body itself (state was null in the runs list at claim time; no post-merge PR content in events), and the tracker rows beyond what the planner's `sd ready`/`sd show` outputs exposed — so seed coverage claims above rest on that listing (200 seeds scanned by title).
