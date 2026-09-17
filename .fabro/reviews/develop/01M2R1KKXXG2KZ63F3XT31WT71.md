# Improve review — run 01M2R1KKXXG2KZ63F3XT31WT71

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (39.5 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-17 17:04+0000 by revisor `fabro_ask`

---

# Recommendations for run 01M2R1KKXXG2KZ63F3XT31WT71 (seed fabro-4ebd, all stages green, ~39.5 min / $2.39)

Grounding used: stage timings and usage from run events/conclusion, stage journals (`.fabro/journal/01M2R1KKXXG2KZ63F3XT31WT71.jsonl`), the planner/implementer/reviewer transcripts, and the `sd ready` listing visible at planner seq 38–39.

**Cost profile (from run events):** implementer 33.0 min wall / $2.216 = **93% of run cost**, 80 tool calls; planner 56 s / $0.109; reviewer 44.6 s / $0.063; tester 233 s. Everything below targets the implementer's dominance or the loop machinery around it.

---

1. **Kill the stale-nextest-binary false-red class in the verify/gate scripts.** Evidence: implementer journal — nextest ran pre-edit test binaries *twice* this run; union + wire tests "failed" with pre-fix behavior, passed after `touch <file>`; "~4 wasted debug cycles" inside the cost-dominant stage (recorded as mx-a74829; same class as mx-1e5205 earlier the same day — recurring). Change: in `scripts/verify.nu` (and the `justfile` qualitygate recipe's nextest leg), force a rebuild of touched crates when source mtime > test-binary mtime (touch sources or `cargo clean -p <crate>` before `cargo nextest run`). Expected effect: eliminates a recurring false-failure loop that burns debug cycles in every run touching fabro-server tests. **New-seed justification:** only workaround lessons (mx-1e5205, mx-a74829) exist; no seed in the tracker covers the verify/gate-side rebuild fix.

2. **Sweep reviewer "noticed but did not block" findings into seeds before closeout — seed fabro-22fa (open).** Evidence: reviewer journal this run flagged two real gaps in the code *just landed*: no pagination on `compare_entries`/`list_pull_request_file_statuses` (per_page=100, >100-file diffs slip past the new merge gate — the exact safety net of arm (c)) and staged blobs hardcoding mode 100644. Both died with the closed seed. Change: implement fabro-22fa in `.fabro/workflows/develop/scripts/closeout.nu` (re-file reviewer-journal non-blocking findings as seeds). Expected effect: the merge gate's >100-file blind spot becomes tracked work instead of a journal footnote on a closed Critical.

3. **Stop anchoring acceptance bullets at unverified code seams — seed fabro-1314 (open).** Evidence: the brief's criterion (b) said "verify via the publish flow in `publish.rs`", while the seed body itself carried the forensics pointing at the supervisor merge commit (452dd28cd, `pull_request_conflict.rs`); the implementer had to run git forensics to disprove the planner's hypothesis (implementer observation) — all inside the 33-min stage. Change: `.fabro/workflows/develop/prompts/planner.md` step 6/7 — acceptance bullets state outcomes only; when the seed body names incident commits, the brief must cite them and location pointers stay in the labeled-hypothesis bullet. Expected effect: implementer starts at the verified seam; forensics re-derivation removed from the dominant stage.

4. **Cut the `sd ready` firehose from the planner lap — seed fabro-c3b4 (open).** Evidence: planner seq 38 — `sd ready --assignee fabro --limit 200` returned 200 lines / 28.9 KB with stdout truncated, just to re-derive the top-5 ordering that `output.preflight` *already* delivered inline (fabro-4ebd, 0d48, 53d3, 8d30, b7c4). Change: planner prompt step 1 — treat `output.preflight`'s candidate table as the ordered candidate source; fetch `sd show` only for the chosen seed. Expected effect: one fewer large tool round-trip and less planner context per pass.

5. **Make the evidence capture reviewable without a blob detour — seed fabro-cf3e (open).** Evidence: this run's capture was 45.0 KB (evidence stage output_bytes) → blob-ref'd despite `preamble_budget_kb=48` being raised exactly to keep per-seed captures inline (fabro-1e9f); the reviewer burned its only 2 tool calls paging blob `d2a0e8f5…` and journaled it as a painpoint. Change: `.fabro/workflows/develop/scripts/evidence.nu` — emit a per-file section index (path, adds/deletes, offset) in the preview head, and raise the reviewer node's `preamble_inline_max_kb` per fabro-cf3e (16 KB cannot hold even a 32 KB capture; the index makes any size navigable). Expected effect: reviewer verifies from context in one pass; blob paging becomes the exception.

6. **Backfill `pull_request.state` for terminal runs — seed fabro-4bb7 (open).** Evidence: planner seq 41 — succeeded runs 01M2QY7N5QZ… (PR #219) and 01M2QWF2RX… (PR #217, actually merged) both show `state: null`; the in-flight guard only trusts `"open"`, so the planner had to reason around nulls (its reasoning trace does so explicitly). Change: populate the state field in the `fabro_runs_list` projection for terminal runs. Expected effect: the double-pick guard (fabro-22e4 class) stops depending on the model correctly interpreting null.

7. **Deduplicate `ml record` output.** Evidence: implementer diff on `.mulch/expertise/engine.jsonl` — two records for the same lesson 7 s apart: mx-a74829 (full) and mx-c83b9d (stub "nextest-stale-binary"), while `lesson_capture` names only mx-a74829. Change: implementer prompt step 6 (and/or `ml record` itself) — one record per lesson; a correction amends, never re-files. Expected effect: expertise store stops accreting stub duplicates that future `ml search` hits return instead of the real record. **New-seed justification:** no existing seed covers ml-record dedup (fabro-17df covers *requiring* records, not duplication).

8. **Surface per-stage cost concentration in the PR body — seed fabro-1409 (open).** Evidence: this run looks uniformly "green", but run events show one stage at 93% of cost and 83% of wall; nothing in the user-facing PR/summary surfaces that. Change: implement fabro-1409 (append per-stage cost/time table + journal painpoint digest to the PR body). Expected effect: the user sees immediately which stage (and which painpoint, e.g. #1) dominates a "successful" run.

---

Not inspected: the tester's raw gate log content beyond its 517-byte summary (blob), and the implementer's single errored shell call's exact payload — the stale-binary false-reds are documented in the journal, but the individual failing command text was not retrieved.
