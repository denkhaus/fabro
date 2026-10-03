# Improve review — run 01M25K41V4NEWXSA3SVGRHKRXK

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (55.4 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-10 13:04+0000 by revisor `fabro_ask`

---

All recommendations below are grounded in this run's events, stage timings, billing, and journal. Run shape for context (from the run conclusion and stage outcomes): 55 min wall, $3.11 total — implementer 43.9 min wall / $2.89 (93% of cost), tester gate 7.6 min, planner 2.1 min, reviewer 57 s / $0.068, evidence 0.8 s, closeout 0.3 s. Zero retries, one-pass approval — so these are optimizations of the *happy path*, not failure cleanup.

## 1. Stop the implementer from re-running the full suites the gate re-runs anyway
**Evidence (run events):** after correctly running the new tests by name filter (seq 493, 510, 645), the implementer then ran the FULL crate suites at seq 666–685 (`cargo nextest run -p fabro-types -p fabro-store`, `-p fabro-store`, `-p fabro-workflow`, `-p fabro-server` — the 1,545-test server suite alone), ~12:30→12:40, plus re-runs at seq 807/813 after fmt (seq 801). Nine minutes later the tester re-ran fmt + clippy + nextest on the *identical* 4-crate set (gate output, 457 s). That is ~10–15 min of the 43.9-min stage spent duplicating a deterministic step.
**Change:** `.fabro/workflows/develop/prompts/implementer.md`, step 4 — replace "full crate suite for every test-file-touched crate" with the open-seed-fabro-7f58 rule: run only NEW/edited test names by filter; full crate suite only when *existing* tests were edited (regression risk). Crate-scoped fmt/clippy stays (it keeps the gate compile-warm — that's what the comment in `workflow.fabro` actually claims).
**Expected effect:** ~10–15 min (≈20% wall) off every green Rust run; implementer tool time (1,293 s here) drops toward the 457-s gate-warm floor.

## 2. Kill the reviewer evidence-blob detour (third recurrence)
**Evidence (run events + reviewer journal):** evidence capture was 62,474 bytes; reviewer node has `preamble_inline_max_kb=16` and graph `preamble_budget_kb=24`, so it arrived as a blob ref. The reviewer's journal painpoint (12:56:51): the first `read_file` landed mid-file truncated (~14 KB omitted), recovered only via `nu -c 'open --raw … | str substring'`. Open seeds fabro-8d2c / fabro-cf3e / fabro-a0fe track the same symptom.
**Change:** two-part, in `.fabro/workflows/develop/workflow.fabro` (reviewer node) and `prompts/reviewer.md`: (a) since 62.6 KB will never fit any sane inline budget, stop tuning budgets — instead make the blob-ref marker state the byte size, and rewrite the reviewer prompt's "LARGE VALUES" paragraph into a mandatory first action: `nu -c 'open --raw <path> | str substring 0..20000'` *before* any `read_file` (exactly the reviewer's own suggested fix); (b) `preamble_inline_max_kb=16 → 32` so smaller captures (~14 KB seen historically) stay inline whole.
**Expected effect:** one-shot evidence reads; removes the verification-blocked bounce risk (an unread blob ref is grounds for a full re-capture + re-review cycle ≈ 2–4 min + a second reviewer visit) and the reviewer's only wasted tool call of this run.

## 3. Give the planner the footgun guard the implementer already has
**Evidence (run events, planner seq 45–89):** the basis check burned ~8 shell calls on avoidable errors: seed's wrong path probed literally (`lib/foundation/fabro-store/...` → IO error, seq 46); `fd: command not found` (seq 52 — fd isn't installed); and seq 63 is a triple footgun: `rg -rn "mark_run_running" … --include='*test_support*' 2>/dev/null` — `rg -rn` is the replace-flag bug documented *only in implementer.md*, `--include` is a grep flag, and `2>/dev/null` swallowed the error so the whole clause silently no-op'd. It then needed three more calls (seq 69–89) to find `mark_run_running` in the other crate.
**Change:** `.fabro/workflows/develop/prompts/planner.md` — add the three one-liners the implementer prompt already carries plus an available-binaries line (open seeds fabro-6997, fabro-0586): never `rg -rn` (`-r` = replace); no `fd`, use `rg --files`/`glob`; never `2>/dev/null` on probes (fabro-b6f9); `run_state.rs` lives at `lib/components/fabro-store/src/run_state.rs`.
**Expected effect:** planner stage 2.1 min → ~1.5 min; more importantly, a swallowed probe error can make a valid basis *look* stale and trigger a wrong superseded-close — the error handling here is currently luck, not guard.

## 4. Trim the `sd ready` firehose
**Evidence (run events, seq 31–33):** `sd ready --assignee fabro --limit 200` returned 27.5 KB (200 lines) — flagged `stdout_truncated: true` — to pick ONE seed. That is ~7k tokens of dead weight in every planner conversation.
**Change:** `prompts/planner.md` step 1: keep `--limit 200` (the fabro-c16d truncation rule) but pipe through `head -40` in the shell; the listing is priority-ordered, and only the top unblocked entries are ever claim candidates (open seed fabro-c3b4).
**Expected effect:** smaller planner context on every run; near-zero risk since lower entries are only needed when the top is skipped, in which case the planner can widen.

## 5. Make in-flight self-exclusion explicit instead of inferred
**Evidence (run events, seq 41–44):** `fabro_runs_list` correctly returned this run itself as the only non-terminal develop run; the planner self-excluded by *reasoning* ("the only non-terminal run is this run itself"), not because the prompt says so — planner.md step 4 never mentions the run's own id (present in its prompt header). It worked here; a future pass that fails the inference would skip its own top candidate or journal a phantom `skipped: in-flight run`.
**Change:** `prompts/planner.md` step 4 (IN-FLIGHT check): one sentence — "always exclude your own run id (first line of this prompt) from the in-flight set before evaluating candidates."
**Expected effect:** deterministic guard replacing model inference on a check that runs on every develop pass; closes part of open seed fabro-6b58.

## 6. Add a long-stage heartbeat for run visibility
**Evidence (run events):** the implementer stage was silent for 43.9 minutes (12:04→12:48); notifications fire only on `run.completed`/`run.failed` (run settings). A user watching Slack/#dev-fabro or the web UI had no signal between "planner done" and "gate green".
**Change:** run settings `notifications` (or open seed fabro-b769): emit a heartbeat event/notification when a stage exceeds, say, 15 min — the engine already has per-stage wall time at checkpoint time, so this is a threshold check, not new instrumentation.
**Expected effect:** during the dominant 80%-of-wall stage, the line's operator can distinguish "working" from "hung" without opening the run — the exact blind spot this run exposed.

One non-issue worth noting so it isn't "fixed": the 62.6 KB gate→blob→`command.output` plumbing and the `closeout`/`evidence` deterministic nodes performed exactly as designed (0.3 s / 0.8 s, zero retries), and the gate timeout of 20 min is comfortably right for a warm 7.6-min run with 15-min cold headroom.
