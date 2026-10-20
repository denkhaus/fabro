# Improve review — run 01M5CSS93MRRW1N82N5SQ26BVM

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (10m10s, revisor pass — reason and cost in run detail)
- generated: 2026-10-20 14:15+0000 by revisor `fabro_ask`

---

Recommendations below are grounded in this run's events (stage transcripts, tester verdicts, journal entries, per-stage timing/cost from the run conclusion). Dedupe searches I ran against the tracker: `seed-spec-paths`, `preflight in-flight`, `in-flight check window self-exclusion`, `gh CLI`, `planner-preflight gh open-PR branch scan`, `preflight server PR tables degraded`, `source probe FABRO_STAGE`, `hide main def-main`, `nextest name filter zero tests`, `bdff`, `top-N ready firehose`, `loop-gate tier order cheapest`, `battery gate cheap tier implementer verdict`, `prompt-lint warnings date pin`, `heartbeat notification visibility`.

Run baseline: 609.9s wall, $0.538 total; planner 180.6s/$0.202, implementer pass 1 208.9s/$0.172, implementer pass 2 97.3s/$0.105 (from run conclusion).

## 1. Run the seed-spec-paths check at implementer time (highest impact — it would have saved ~20% of this run)
- **What happened**: implementer@1 touched only 2 of the 3 seed-named paths (`scripts/verify.nu`, develop `implementer.md`), journaled "planner.md step 6 carries no full-suite language today, so no mirror edit was needed" — directly contradicting brief bullet (4) — and routed `Implemented`. The omission surfaced only at tester@1's seed-spec-paths tier (`planner.md — NOT in diff`), costing a full red gate (15.1s) plus a full bounce pass (implementer@2: 97.3s, $0.105, 29,759 input tokens ≈ 20% of run wall and cost). The implementer@2 journal painpoint proposes exactly this fix.
- **Change**: extract the seed-spec-paths tier from `.fabro/workflows/loop/scripts/loop-gate.nu` into an invocable check (the way `nu scripts/qualitygate.nu check-run-scope loop` is offered) and add it to the loop implementer prompt's verification lane (`.fabro/workflows/loop/prompts/implementer.md`, step 4) as a pre-routing verdict.
- **Expected effect**: one-unit omissions are caught before the gate; eliminates the gate-red bounce class (~110s + ~$0.11 per occurrence here).
- **Seed**: no open seed — only closed fabro-48f3 (different scope: worktree tokens/add-windows). *New-seed justification: this run's implementer@2 journal painpoint proposes it and the only seed-spec-paths seed (fabro-48f3) is closed and covers a different defect.*

## 2. Guard the new name-filter arm against zero-match false greens
- **What happened**: the reviewer's journal observation (and the residual filed by closeout as **fabro-bdff**): "when nextest receives a name filter that matches zero tests in the crate, exit semantics depend on nextest's no-tests handling; an extraction false-positive … could in principle produce a green-looking 0-test run."
- **Change**: in `scripts/verify.nu`, the `names` arm of `test-mode-for` should fail or widen to the full suite when the nextest run reports 0 tests matched (e.g. parse the run summary, `0 tests run` → widen).
- **Expected effect**: closes the only correctness hole the reviewer found in this run's shipped change.
- **Seed**: **fabro-bdff** (open, @loop, residual) — filed by this run's closeout; prioritize it.

## 3. Make the preflight in-flight flag exclude terminal runs
- **What happened**: preflight flagged fabro-8932 `in_flight` pointing at run `01M4NV2HNFF0KGWZ8BF23J375C`, which the planner found is failed+archived (planner journal: "flag is stale once a run terminal-fails"). Adjudicating that stale flag burned 4 `fabro_run_search` calls (1 errored) plus reasoning inside the run's longest, most expensive stage (planner: 180.6s, $0.202, 29 tool calls).
- **Change**: in `.fabro/workflows/develop/scripts/planner-preflight.nu`, only count non-terminal, non-archived runs as in-flight.
- **Expected effect**: removes a recurring adjudication detour in the planner; stale flags stop outranking genuinely claimable seeds.
- **Seed**: **fabro-6b58** (open, @loop: created_since window + self-exclusion) is the nearest owner — fold the terminal-status exclusion into it (fabro-1437, open/unassigned, covers the adjacent orphaned-branch variant). A time window masks but does not fix the status defect, so extend the seed rather than file a sibling.

## 4. Reorder loop-gate tiers cheapest-first
- **What happened**: on the red cycle, tester@1 ran the full battery first — validate-workflows (25 graphs), lint-nu (67 scripts), prompt-lint (81 files), run-scope, mode-preservation, dot-snapshot — and only then REDed on seed-spec-paths, a diff-vs-seed-body check that needs no toolchain (tester@1 output order, 15.1s).
- **Change**: in `.fabro/workflows/loop/scripts/loop-gate.nu`, move the seed-spec-paths (and run-scope) tiers ahead of validate-workflows/battery-runner.
- **Expected effect**: gate-red bounces fail in <1s instead of ~15s, and red output stops being buried under ~7KB of battery noise.
- **Seed**: none found (`loop-gate tier order cheapest`, `battery gate cheap tier implementer verdict` → 0). *New-seed justification: no existing seed addresses gate tier ordering; the cheaper-tier-first rule exists for planner probes but not for the loop gate.*

## 5. Replace the gh-shelling open-PR arm in preflight with a server-table query
- **What happened**: planner journal painpoint: "gh CLI absent in planner shell, so the open-PR arm of the in-flight check ran degraded (branch scan + base grep only)… Fix idea: teach planner-preflight.nu to query the server's run/PR tables instead of shelling gh." The degraded arm weakens the duplicate-run protection the lane relies on.
- **Change**: `.fabro/workflows/develop/scripts/planner-preflight.nu` — source PR/run state from the Fabro server tables (the engine already exposes run search as a host tool) instead of `gh`.
- **Expected effect**: the in-flight check stops silently degrading on every run; claim-race protection is deterministic.
- **Seed**: none found (`gh CLI`, `preflight server PR tables degraded`, `planner-preflight gh open-PR branch scan` → only fabro-ea68, an unrelated UI seed). *New-seed justification: this run's planner painpoint names the fix and no tracker seed covers the preflight's gh dependency.*

## 6. Fix the prescribed probe form that executed a real verify stage by accident
- **What happened**: implementer@1's recorded check shows the prescribed `nu -c "source scripts/verify.nu"` probe actually ran `stage-implementer` because `FABRO_STAGE` was set — the transcript captures a spurious `cargo fmt` failure against the /tmp fixture ("`cargo metadata` exited with an error: could not find Cargo.toml"). Implementer@1 journaled the workaround (set `FABRO_STAGE=''` or `hide main`), but the prescription itself lives in the planner prompt ("Implementer pre-gate scoping … `nu -c \"source <new-or-edited script>\"`").
- **Change**: in `.fabro/workflows/loop/prompts/planner.md` (and the develop mirror), change the prescribed single-file probe to `nu -c "hide main; source <script>"` (or prefix `FABRO_STAGE=`).
- **Expected effect**: probes stop silently running a real cargo pass; recorded-check transcripts stop carrying misleading FAIL output the reviewer must then adjudicate (the reviewer here had to note "fmt-fail output in the probe transcript is from a temp fixture … not a real verdict").
- **Seed**: **fabro-ee1f** (open, @loop) covers the lint-nu rule for repo scripts that source def-main; it does not cover the prompt prescription, and the offending probes live in /tmp where lint-nu never runs — *new-seed justification: the fix is a prompt-prescription change; fabro-ee1f only adds a lint rule and cannot see /tmp probe files.*

## 7. Ship the top-N ready view (planner context trim)
- **What happened**: the planner's first tracker call (`seeds ready --assignee loop --limit 200`) dumped all 56 ready seeds (~8.1KB) into context (event stream seq 92–95), even though preflight had already computed the 5 top candidates.
- **Change**: implement **fabro-c3b4** (open, @loop) — top-N ready view in the planner instead of the firehose.
- **Expected effect**: smaller planner preamble and less attention spent scanning 51 irrelevant rows in the run's most expensive inference stage.

## 8. Long-stage heartbeat + lint-warning triage (UX)
- **What happened**: notifications are terminal-only, so the run was silent through planner (180.6s) and implementer@1 (208.9s) — a ~3.5-minute mid-run blind spot; separately, every gate run re-emits 35 prompt-lint warnings (17 date-pin, 15 routing-named schema warnings) plus 41 synthetic-id notes, duplicated across tester@1, tester@2, and the evidence capture (28KB).
- **Change**: implement **fabro-b769** (open, @loop: long-stage heartbeat notification) for the visibility gap; file a warning-triage pass for prompt-lint (summarize recurring warnings once per run instead of three times).
- **Expected effect**: mid-run visibility for watchers; ~40% smaller tester/evidence output and a cleaner reviewer preamble.
- **Seed**: heartbeat = **fabro-b769**. Warning triage: none found (`prompt-lint warnings date pin` → 0; fabro-0d16 covers only the SYNTHETIC_IDS marker drift) — *new-seed justification: no existing seed covers recurring-warning triage in gate/evidence output; fabro-0d16 fixes one warning class, not the duplication across stages.*

Not inspectable from here: whether the loop lane's `prompts/implementer.md` and `planner.md` carry these paragraphs verbatim (loop assets are workspace files; the graph and stage transcripts reference them, and I did not open each prompt file). The stage transcripts and gate outputs above are from run events; timings/cost from the run conclusion.
