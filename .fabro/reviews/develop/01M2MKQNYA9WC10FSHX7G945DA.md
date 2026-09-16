# Improve review — run 01M2MKQNYA9WC10FSHX7G945DA

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (3.6 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-16 08:09+0000 by revisor `fabro_ask`

---

All evidence gathered (run events seq 1–199, worker log, stage transcripts, cost/timing data). Basis for what follows: this run executed seed **fabro-e702** end-to-end in 191.6s wall / $0.173 total (planner $0.069, implementer $0.071, reviewer $0.033), 0 retries, gate green in 3.6s, PR #165 with auto-merge. Recommendations ordered by expected impact:

---

**1. Cap the planner's `sd ready` payload — seed fabro-66bc** ("Planner: use a top-N priority-sorted sd ready invocation")
- **Change:** `.fabro/workflows/develop/prompts/planner.md` PROJECT_FACTS command table — replace `sd ready --assignee fabro --limit 200` with a priority-sorted top-N form (~10 candidates).
- **Evidence:** from run events seq 30 — the call returned **200 seeds / 27,698 bytes**; the planner's first LLM turn billed 14,975 input tokens (cache_read 0) largely on that listing, yet it only needed the top High candidate. Planner = 40% of run cost ($0.069/$0.173) and 30% of wall (58.2s).
- **Effect:** ~6k uncached input tokens + one pass of listing-comprehension removed per run; planner turn-1 shrinks ~40%; also reduces mispick risk from the planner skimming 200 rows.

**2. Make the in-flight guard trustworthy: backfill PR state + bound the check — seeds fabro-4bb7 + fabro-6b58**
- **Change:** engine projection backfill of `pull_request.state` for terminal-but-unmerged runs (fabro-4bb7); planner's `fabro_runs_list` call gains `created_since` + explicit self-exclusion (fabro-6b58).
- **Evidence:** from run events seq 33/36 — prior run 01M2MJ0X shows PR #163 `"state": null` despite `status: succeeded`; the planner's reasoning trace explicitly deliberated "state null but run succeeded — not in flight" and journaled it as its **first observation** (checkpoint seq 61). The `fabro_runs_list` call (seq 32) passed no `created_since`, returning 13 runs back to 2026-09-14 including the run itself, forcing manual self-exclusion.
- **Effect:** eliminates the residual double-pick window (the fabro-22e4 family — a whole wasted run when it fires) and removes ~10s of per-run guard deliberation; the planner stops re-deriving what the projection should state.

**3. Scope per-node memory — seed fabro-9588** ("Per-node skills and memory scoping for agent stages")
- **Change:** engine session init — load `AGENTS.md` selectively per node instead of universally.
- **Evidence:** from run events seq 21, 67 and the reviewer session — `AGENTS.md` (25,271 bytes) was injected into **all three** agent sessions (memory tokens 6,614 / 6,251 / 6,339 ≈ 19.2k tokens/run). The reviewer and planner don't need the full coding-convention memory to route and verify.
- **Effect:** ~6k tokens × 2 stages saved per run (~12k of 305k billed), smaller contexts, lower prompt-cache pressure; reviewer input drops from 18.9k toward ~12.5k tokens.

**4. Make the implementer's spec re-fetch conditional — seed fabro-a67f** (High; narrower variant fabro-4881)
- **Change:** `.fabro/workflows/develop/prompts/implementer.md` step 1 — re-run `sd show` only when the brief is thin, not mandatorily.
- **Evidence:** from run events seq 75–77 — the implementer's first action re-fetched `sd show fabro-e702`, returning the **byte-identical description** the planner had already distilled into `current_seed_brief`. That turn billed 18,451 input tokens / $0.026 for zero new information.
- **Effect:** one fewer LLM turn per implementer pass (~6s, ~$0.02–0.03/run); on this run that's ~9% of implementer stage cost.

**5. Batch planner reconnaissance into one shell call — seed fabro-55a7** (near-duplicate fabro-2be2 — consolidate the two)
- **Change:** `.fabro/workflows/develop/prompts/planner.md` — combine `sd show` + stale-basis grep (+ claim) into one shell invocation.
- **Evidence:** from run events seq 29–51 — the planner ran 5 strictly sequential tool calls (`sd ready` → `fabro_runs_list` → `sd show` → `grep` → `sd update`) with an LLM turn between each, ~30s of the 58s stage. The `sd show` + grep pair (seq 37–45) is trivially batchable.
- **Effect:** 2–3 fewer LLM turns ≈ 15–20s and ~$0.015/run; also note the tracker carries two seeds (55a7, 2be2) for the same change — close one to avoid double-filing.

**6. Fix the pipeline-progress counter — seed fabro-45bf** ("Compute pipeline progress from unique completed nodes", High)
- **Change:** engine stage-prompt renderer — count unique completed nodes.
- **Evidence:** from stage prompts — implementer@1 opened with "Pipeline progress: **0 of 7** stages completed" when start+planner were done; reviewer@1 said "**2 of 7**" with four stages completed (run events seq 65, 139).
- **Effect:** honest progress in every stage prompt (agent self-orientation + user-visible accuracy) at zero cost; a 4-line change.

**7. Make PR-body generation non-strict — seed fabro-41b1** ("PR postlude: make PR-body generation non-strict — strict-JSON parse fails, retry masks it")
- **Change:** `lib/components/fabro-workflow/src/pipeline/pull_request.rs` — drop the strict-JSON first attempt.
- **Evidence:** from the worker log — 08:04:34 WARN "PR content structured generation failed; retrying once without strict JSON output … model did not return a JSON document". This sits inside the 26s exit→PR window (exit 08:04:24 → PR #165 08:04:50) and fires as a warning on an otherwise clean run.
- **Effect:** one fewer recurring failure+retry per run (~5–10s of postlude) and a clean warn-level log for green runs.

**8. Cite acceptance-bullet numbers, don't restate them — seed fabro-ad1b**
- **Change:** `.fabro/workflows/develop/prompts/implementer.md` inline-verification-report section — `PASS (a): implementer.md:25` style instead of quoting the criterion.
- **Evidence:** from checkpoint seq 120 — the implementer's `implementation_summary` (1,522 output tokens) restated every acceptance criterion verbatim in its PASS lines; that summary then rides into the reviewer preamble.
- **Effect:** ~1–1.5k fewer output tokens per pass and a smaller reviewer preamble; on this run the whole verified diff was 4 added lines.

**9. Downgrade the by-design allow-key warn — seed fabro-8275** ("Downgrade the by-design preamble allow-key absence warn to info")
- **Change:** `fabro_workflow::lifecycle::fidelity` warn site — `output.gate_known_bug_hits` is absent by design on green first visits.
- **Evidence:** from the worker log 08:02:13 — "preamble_allow_keys entry absent from context node=implementer key=output.gate_known_bug_hits" fired on a run where the gatebounce node never ran (gate green, seq 128).
- **Effect:** warn-level noise reserved for real anomalies; makes the 8-line log (6 of which are item 10) actually readable.

**10. Stop logging the missing optional memory file as ERROR — new-seed justification:** no existing seed covers this site (fabro-8275 covers only the preamble allow-key warn; nothing in the ready list addresses `.codex/instructions.md` read errors).
- **Change:** `coding_session_initialize` file-read path — treat an absent optional memory file as info/debug, not ERROR.
- **Evidence:** from the worker log — **6 ERROR lines** (2 per agent session init, planner/implementer/reviewer) for `File "/workspace/fabro/.codex/instructions.md" was not found` on a fully successful run.
- **Effect:** 6 fewer ERROR lines per run; error-level signal stays reserved for real failures, which matters for log-based triage of genuinely broken runs.

---

**What worked and needs no change (from this run):** the churn-only evidence path (4.6KB capture rendered inline — no blob detour, so fabro-8d2c/fabro-cf3e budget raises would be premature here), the deterministic closeout via `stdin_source=current_seed_id` (closed exactly fabro-e702, 354ms), the per-seed diff base (evidence header `diff-base=c92fbc3`), and the gate's loop-asset short-circuit (3.6s "no crates touched"). The implementer's `timeout_ms >= 60000` rule added by this very seed should also cut the silent-timeout failure class it cites going forward.
