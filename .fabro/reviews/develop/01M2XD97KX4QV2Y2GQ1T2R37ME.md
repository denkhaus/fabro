# Improve review — run 01M2XD97KX4QV2Y2GQ1T2R37ME

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (11.6 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-19 18:17+0000 by revisor `fabro_ask`

---

All recommendations below map to **existing open seeds** — I checked `.seeds/issues.jsonl` for coverage before recommending (no new-seed filings needed). Evidence comes from this run's events/checkpoints: total 671 s wall / $0.666, planner $0.317 (48%), implementer $0.301 (45%), reviewer $0.048; seed fabro-3196, PR #291.

## Ordered by expected impact

**1. Kill the `sd ready` firehose in the planner — seed fabro-c3b4 (open, @fabro)**
What happened: planner@1's `sd ready --assignee fabro --limit 200` returned **28,512 bytes / 200 issues, `stdout_truncated: true`** (event seq 46), yet the preflight had already narrowed the field to 5 candidates. That ~7k-token blob sat in the conversation for the remaining ~12 planner turns.
Change: `.fabro/workflows/develop/prompts/planner.md` step 1 — replace the full `sd ready --limit 200` call with a top-N view (the preflight table already carries the top 5); keep `--limit 200` only for the degraded-mode fallback.
Effect: cuts the planner's dominant input token mass and misread risk from a truncated listing; direct lever on the planner's 48%-of-run-cost share.

**2. Bound `fabro_runs_list` with `created_since` — seed fabro-6b58 (open)**
What happened: the pre-claim in-flight check (event seq 48–49) listed **91 runs back to ~04:00** and took **12.3 s**; the planner then had to reason "no open non-terminal runs other than this one" over the whole dump (reasoning trace seq 52).
Change: `.fabro/workflows/develop/prompts/planner.md` step 4 — mandate `created_since` (e.g. 24 h window) plus explicit self-exclusion on the `fabro_runs_list` call; non-terminal runs older than the window are re-checked individually only if the tracker shows a matching in_progress seed.
Effect: ~12 s latency off the critical path and a much smaller payload; the check stays sound because stale claims are already requeued by tracker_guard's 6 h arm.

**3. Batch planner reconnaissance into one shell call — seed fabro-55a7 (open)**
What happened: the planner's basis verification ran as **~10 sequential single-purpose shell calls** (grep child_run_id → git log → sed workflow.fabro → grep output_schema → grep jsonschema → ls tests → grep tests…, events seq 59–97), 15 LLM turns total at ~9 s/turn.
Change: `.fabro/workflows/develop/prompts/planner.md` step 3 — the stale-basis check collapses into one labeled compound probe (`grep … ; echo --- ; sed -n … ; echo --- ; git log …`) per candidate, as the seed already specifies.
Effect: 4–6 fewer planner turns (~40–60 s wall) per run; cheaper re-sends of the (cached but billed) conversation.

**4. Raise the reviewer's `preamble_inline_max_kb` 16 → ≥40 — seed fabro-cf3e (open; target already amended to ≥40)**
What happened: the evidence capture was **20.9 KB → blob ref** in the reviewer prompt (reviewer@1 preamble marker), despite the graph's 48 KB aggregate budget — because the **per-node cap is 16 KB** (workflow.fabro reviewer attrs). The reviewer burned its only two tool calls (1 read_file, 1 grep) paging the blob it should have had inline.
Change: `.fabro/workflows/develop/workflow.fabro`, reviewer node: `preamble_inline_max_kb=16` → `40`.
Effect: one fewer tool round-trip per review and removes the standing "unread blob ref" rejection risk in this capture-size class; this run is fresh confirming evidence for the seed.

**5. Add the run-id hygiene clause to implementer.md — seed fabro-41de (open; remaining demand (c))**
What happened: the implementer transcribed full 26-char run-id literals from the seed body into `develop-leg.md`, and prompt-lint **red-lined 4 errors on first pass** (implementer journal + mx-efc7c1), costing a rewrite plus 2 of its 28 shell calls ending in error. The lint caught it at verify time — the editor was never told at edit time.
Change: `.fabro/workflows/develop/prompts/implementer.md` "loop-asset carve-out" section — one clause: run-id/PR-number/sha literals are forbidden in workflow assets; cite the seed id instead (provenance lives in the seed Basis). This is exactly fabro-41de's unlanded demand (c).
Effect: eliminates the guaranteed lint-fail-then-rewrite cycle on every prompt-editing seed (~1–2 min + 2 wasted calls this run).

**6. Sweep reviewer "noted but not blocking" findings at closeout — seed fabro-22fa (open; was this run's own #5 preflight candidate)**
What happened: reviewer@1's journal emitted a concrete hardening finding — *"`conductor_develop_schema_probe` hardcodes expected_labels instead of deriving them from workflow.fabro edges, so a future edge rename would leave the enum pin stale"* — and because the verdict was Approved, the run exited one stage later and that finding dies in `.fabro/journal/…jsonl`.
Change: `.fabro/workflows/develop/scripts/closeout.nu` — before `sd close`, grep the reviewer journal entry for unactioned observations and file them as seeds (the sweep arm fabro-22fa specifies).
Effect: review-adjacent improvements stop evaporating; this run supplies a live instance (the enum-drift pin) that currently has no carrier.

**7. Backfill `pull_request.state` for terminal-but-unmerged runs — seed fabro-4bb7 (open)**
What happened: in the in-flight exclusion, several **succeeded runs showed `pull_request.state: null`** (PRs #289, #287, #284 in the seq-49 tool output); the planner correctly but uneasily hand-waved "treated as not-open, not blocking" (its journal calls it a "projection gap"). A `closed_unmerged` PR misread here is a false-negative in a correctness check.
Change: engine projection (per fabro-4bb7) so `fabro_runs_list` resolves terminal runs' PR states instead of returning null.
Effect: the in-flight check becomes mechanical instead of a judgment call; removes the planner's documented ambiguity.

**Not recommended (measured healthy this run):** the brief format (bulleted, cheapest-first, labeled hypothesis, timeout floors — planner followed every rule), the tester (23.9 s, "no crates touched" path), evidence ordering, and reviewer economy (34 s, $0.048, 2 tool calls).

**What I could not inspect:** the exact text of the implementer's 2 errored shell calls (I did not page the implementer stage event bodies past seq 149; the prompt-lint attribution comes from its journal and mx-efc7c1, which are consistent with the tool stats).
