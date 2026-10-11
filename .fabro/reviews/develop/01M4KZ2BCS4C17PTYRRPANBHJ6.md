# Improve review — run 01M4KZ2BCS4C17PTYRRPANBHJ6

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (20m31s, revisor pass — reason and cost in run detail)
- generated: 2026-10-10 23:25+0000 by revisor `fabro_ask`

---

# Recommendations for the `Develop` workflow — grounded in run 01M4KZ2BCS4C17PTYRRPANBHJ6 (seed fabro-9f4a, succeeded, 20m31s wall, $0.703 total, 0 retries, PR #392)

Cost/wall breakdown (from run events): planner 272.7s / $0.320 (45% of cost), implementer 473.2s / $0.323 (46%), tester 352.6s / $0, reviewer 43.3s / $0.060, all deterministic guards ≤3.5s / $0. The agent stages are 100% of the cost; the guard/command layer is already optimal.

---

**1. Fix the planner's in-flight exclusion — land fabro-9b84 (open, P2), complemented by fabro-0f6e / fabro-350d (open, engine half)**
- **What happened:** planner@1 = 45% of run cost ($0.32) and 272.7s wall at `reasoning_effort="low"` — 3.3× the wall and 4× the cost of the 83.6s/$0.080 HIGH-effort baseline the graph itself cites (fabro-bbbb comment in `workflow.fabro`). Its own journal painpoint: *"fabro_runs_list tool is absent in this planner environment, so the in-flight exclusion ran degraded (journal grep + branch scan only)"*. Worse: the session tool list (run events, planner stage) contained **no fabro tool at all**, despite the planner node declaring `x.fabro_tools="fabro_run_search"` — and the preflight ran with `"run_id":""` (FABRO_RUN_ID unbound). Both in-flight arms degraded; the planner re-derived exclusions through 26 shell probes across 23 messages (776,896 cache-read tokens billed across those rounds).
- **Change:** in `.fabro/workflows/develop/prompts/planner.md` (step 4, IN-FLIGHT EXCLUSION) rename the dead `fabro_runs_list` references to `fabro_run_search` per fabro-9b84 — and extend fabro-9b84's proposed graph/prompt consistency lint to assert the declared tool actually **materializes in the agent session** (this run proves declaration alone is insufficient; the run-level `agent.fabro_tools=false` in workflow.toml is the likely suppressor — engine precedence could not be inspected from here).
- **Expected effect:** planner probe rounds collapse from ~26 shell calls to one tool call; planner moves back toward the ~84s/$0.08 baseline — roughly $0.15–0.25 and 2–3 minutes saved per run, and the recurring degraded-mode painpoint disappears.

**2. Park unpickable seeds out of `seeds ready` — land fabro-863f (open, P2, assignee fabro), extended to the human-verification residual class**
- **What happened:** 2 of the preflight's 5 candidate slots (fabro-fec1, fabro-0248) are deployed-server human-verification residuals the planner can never claim; it journaled they *"will keep topping seeds ready until re-assigned (same class fabro-863f describes)"* and burned adjudication on them plus fabro-deee/863f/23d4. Concrete damage visible in this run: the actually-claimed seed **fabro-9f4a was outside the preflight's 5-candidate window** — it received **no already-landed/duplicate check** (the fabro-a32f guard) because unclaimable seeds occupied the window.
- **Change:** implement fabro-863f's parking label (its own demand: extend the needs-user semantics so `seeds ready` stops surfacing unpickable work), apply it to fec1/0248, and have `.fabro/workflows/develop/scripts/planner-preflight.nu` filter parked seeds from its candidate window.
- **Expected effect:** the claimed seed always gets the already-landed check (robustness restored), and every planner lap stops re-deriving the same 3–5 skips — compounding with rec 1's savings.

**3. Make `verify.nu` print the failing command's stderr on FAIL**
- **What happened:** implementer painpoint (journal + stage transcript): the first verify invocation printed `FAIL clippy fabro-server` with **completely empty error output** while fabro-automation had failed to compile with E0004 — *"forcing a blind direct `cargo clippy -p` re-run to see it"*. The reviewer independently flagged it (*"a lane script printing 'FAIL clippy' with empty error text cost a diagnosis detour this run"*). Root cause is in `scripts/verify.nu` lines 104–138: every FAIL branch prints only `verify: FAIL <check> (<crate>)` and sets `$env.LAST_VERIFY_FAIL` — no captured stderr.
- **Change:** in `scripts/verify.nu`, run checks through nu's `complete` (the pattern `qualitygate.nu`'s battery tier already uses) and print a bounded tail of the failing command's stderr after each FAIL line.
- **Expected effect:** first red verify carries the real diagnostic — saves one blind cargo re-run plus its LLM round-trip on every red verify, and shortens every gate-red bounce diagnosis.
- **New-seed justification:** no existing seed covers verify.nu's FAIL-path output — fabro-7ce7 (run-context duplication), fabro-a9cc/fabro-6e7f (stage dispatch), fabro-574d (crate mapping) all touch verify.nu but none address stderr capture; this painpoint is from today (expertise entry mx-fdcc45).

**4. Cut prompt-lint's warning noise floor (intentional routing schemas + toolchain date-pins)**
- **What happened:** the tester gate output ends `prompt-lint: ok — 72 files, 20 warnings` (from run events, tester@1). 17 of 20 are "top-level property … is routing-named" fired on **intentional** routing schemas — including the develop workflow's own `schemas/planner-output.schema.json` — and 2 are date-pin warnings on `2026-04-14`, which is the pinned nightly Rust toolchain (load-bearing, not stale prose). The warning was added deliberately (fabro-a211, closed), but now that every workflow's schema is legitimately routing-active, it fires on every gate run and buries real warnings — and the reviewer reads this log every run via the tester section.
- **Change:** in `.fabro/scripts/prompt-lint.nu`, honor an opt-in marker for intentional routing schemas (the exact mechanism fabro-cb5c landed for synthetic fixture ids) and teach the date-pin rule to recognize toolchain pins (`nightly-YYYY-MM-DD` / the `PINNED_TOOLCHAIN` constant verify.nu and qualitygate.nu already share).
- **Expected effect:** gate output drops from 20 warnings to the ~1 real one; every run's gate log stops carrying 19 lines of adjudication noise for the reviewer to read past.
- **New-seed justification:** no open seed covers it — fabro-a211 (closed) introduced the warning, fabro-cb5c (closed) established the marker mechanism for a different check; nothing files the noise-floor reduction.

---

**Considered and rejected:** deduplicating the implementer's `just verify implementer` full-suite run against the tester's identical fmt+clippy+nextest pass (the tester started 0s after the implementer ended on the same tree, so the suite ran twice, ~2 min of the 352.6s tester wall). Rejected because the tester is the trust boundary by design — fabro-6e7f's evidenced gate misses (PR #346: local gate SUCCESS on a non-compiling tree) are exactly why the gate must re-derive rather than trust the lane.

Sources: run events (`fabro_run_events`, stage projections and tester/planner outputs), run journal `.fabro/journal/01M4KZ2BCS4C17PTYRRPANBHJ6.jsonl`, workspace files `scripts/verify.nu`, `scripts/qualitygate.nu`, `.fabro/workflows/develop/workflow.fabro`, and the seed tracker `.seeds/issues.jsonl`. I could not inspect engine precedence for run-level vs node-level `fabro_tools` (the docs corpus path `lab/fabro/fabro/tools.md` listed in the index did not resolve).
