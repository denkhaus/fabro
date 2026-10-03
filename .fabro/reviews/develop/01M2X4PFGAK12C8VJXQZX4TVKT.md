# Improve review — run 01M2X4PFGAK12C8VJXQZX4TVKT

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (4.8 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-19 15:41+0000 by revisor `fabro_ask`

---

# Recommendations for the develop workflow — grounded by run 01M2X4PFGAK12C8VJXQZX4TVKT

**Run baseline (from run events):** one clean pass, zero cycles/retries — tracker_guard 0.4s → preflight 11.5s → planner 61.6s/$0.130 → implementer 154s/$0.200 → gate 8.4s green → evidence 12.9 KB inline → reviewer 19s/$0.029, **0 tool calls** → closeout. Total $0.359 / 4m48s. The single-pass shape works; the findings below are where time, cost, and correctness leaked.

---

**1. Fix the terminal-run `in_flight` false positive — seed fabro-32db (open, Medium·bug)**
- **What happened:** the preflight table (run events, preflight stage) marks `fabro-af22 in_flight: true, in_flight_run 01M2WAY0KHV8…`, but `fabro_runs_list` (planner tool call, seq 49) shows that run **terminal-failed at 08:05:13Z** — 7.5h before this run. tracker_guard returned `requeued: []` despite `stale_threshold_hours: 6.0` (its stdout, seq 20). The claim stranded on an orphaned branch, so **the only High-priority seed not behind an open PR (fabro-af22) is being skipped by every run**, which fell through to Medium-priority fabro-395b. The planner's own reasoning (seq 52) had to argue itself past the contradiction ("failed status; but preflight marked in_flight").
- **Change:** in `.fabro/workflows/develop/scripts/planner-preflight.nu` (in-flight arm) and `.fabro/workflows/develop/scripts/tracker-guard.nu` (requeue arm), adopt one definition — a claim is in flight **only while its run is non-terminal** — plus the pinning unit tests the seed demands.
- **Expected effect:** fabro-af22 becomes claimable next run; the loop stops starving top-priority seeds and stops burning planner reasoning on stale-verdict adjudication.

**2. Cap the planner's `sd ready` view — seed fabro-c3b4 (open, Medium·task)**
- **What happened:** the planner's `sd ready --assignee fabro --limit 200` (seq 46) returned **200 issues / 28,750 bytes, stdout_truncated: true** — yet only the top ~5 candidates were ever consulted, and the preflight table had already carried exactly those five. That firehose is dead context inside the planner's 54.5k-token pass ($0.130 = 36% of run cost, 61.6s).
- **Change:** in `.fabro/workflows/develop/prompts/planner.md` (PROJECT_FACTS sd-command table), replace the full listing with a top-N form (e.g. `… --limit 200 | head -10`), keeping the `--limit 200` rationale only for the fallback full picture.
- **Expected effect:** ~10–15% planner input-token cut per run and elimination of the truncation risk that could one day hide the *real* top candidate at the bottom of a clipped stream.

**3. Dry-run literal verification commands before a brief ships — seed fabro-4be6 (open, Medium·task)**
- **What happened:** the brief prescribed `` `nu -c 'use … ; classify-filed …'` `` probes. The implementer then burned **three failed shell calls** (events seq 144, 151, 158: bash choking on nu syntax → `module not found` from `/tmp` → `Command classify-filed not found` — the function isn't exported), exactly as its journal records ("classify-filed is not exported … the regex had to be probed by duplicating it in a temp script"). Net: 146.8s inference vs 6.8s tool time — 59% of run inference and 56% of cost on a two-file regex change.
- **Change:** add the fabro-4be6 rule to `.fabro/workflows/develop/prompts/planner.md` step 6: every literal probe command named in a brief must be dry-run once in a scratch shell before the brief ships (adjacent hardening: fabro-3805's seed-authoring lint).
- **Expected effect:** implementer stops losing round-trips to impossible commands; on script-seeds like this one that's several wasted calls and a chunk of the 146.8s inference per run.

**4. Scope memory per node — seed fabro-9588 (open, needs-triage)**
- **What happened:** all three agent sessions loaded the full `AGENTS.md` (25,746 bytes ≈ 6.5k tokens each — MemoryLoaded events seq 37, 92, and reviewer session init). The reviewer — which made **zero tool calls** and issued a checklist verdict — spent 6,538 of its 18,208 input tokens (36%) on memory it never used.
- **Change:** land fabro-9588's per-node memory scoping (engine) and declare narrowed memory per node in `.fabro/workflows/develop/workflow.fabro` (reviewer needs none of the implementer-facing AGENTS.md bulk).
- **Expected effect:** ~1/3 off reviewer-type stage inputs and meaningful cuts on planner/implementer; direct cost and TTFT improvement on every run.

**5. Backfill `pull_request.state` for terminal-but-unmerged runs — seed fabro-4bb7 (open, Medium·task)**
- **What happened:** `fabro_runs_list` (seq 49) returned `"pull_request": {"number": 280, "state": null}` for succeeded run 01M2WZHS4MZ8; the planner's reasoning (seq 52) explicitly wrestled with "state null but run succeeded — preflight says in_flight, skip". The in-flight PR check degenerated into inference plus a 12.5s catalog call instead of a one-field read.
- **Change:** implement fabro-4bb7 in the run projection so terminal runs always carry a resolved PR state.
- **Expected effect:** the in-flight exclusion check becomes deterministic; removes both the wasted planner reasoning and the mis-skip/mis-claim risk on freshly-succeeded runs.

**6. Demote optional-file-miss logs to DEBUG — seed fabro-79ca (open, Medium·task, engine)**
- **What happened:** worker log shows **6 ERROR lines this run** — two `File "/workspace/fabro/.codex/instructions.md" was not found` per agent session (planner, implementer, reviewer). On a fully green run, the only ERRORs in the stream are noise.
- **Change:** implement fabro-79ca — optional memory/instruction probes log at DEBUG when absence is an expected branch.
- **Expected effect:** run log streams stop burying real failures; rootprint ERROR sweeps become actionable again.

**7. Stop the per-run `context_allow_keys` drop/warn noise — new seed required.**
*Justification: no existing seed covers engine-written response-dedup keys being dropped by the declared allow-list, nor the expected-absent preamble key warning on green runs (fabro-a341 only covers lint tolerance for `context.`-prefixed reads).*
- **What happened:** run events seq 75 / worker log: `run.notice warn context_update_dropped: context_allow_keys dropped: output.planner` — the engine's response-dedup write is silently dropped by the planner's declared producer contract every pass. Additionally the log warns `preamble_allow_keys entry absent from context node=implementer key=output.gate_known_bug_hits` on every **green** run, where that key is legitimately absent (no gate bounce).
- **Change:** add `output.planner` to the planner node's `context_allow_keys` in `.fabro/workflows/develop/workflow.fabro` (or exempt engine-internal dedup keys from the allow-list check), and mark conditional keys like `output.gate_known_bug_hits` as optional so their absence doesn't warn.
- **Expected effect:** warn-level signal on every run becomes real; the fabro-900e producer-declaration contract stops drifting from actual engine writes.

---

**What to keep as-is (evidence it works):** the churn-only evidence shape (12.9 KB inline, reviewer verified "from the inline loop-work diff only — no blob paging needed"), the deterministic guard chain (tracker_guard 0.4s, claim_check 86ms), and the report-only preflight design — fabro-395b itself just proved the incident class it was filed against is now handled conservatively.
