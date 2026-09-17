# Improve review — run 01M2R8FHE6NQ14SGKXJ015JWPZ

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (30.6 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-17 18:57+0000 by revisor `fabro_ask`

---

All facts below are from this run's events/checkpoints (run 01M2R8FHE6NQ14SGKXJ015JWPZ, seed fabro-8d30 part b): 30.1 min wall, $2.46 LLM total. Stage split — implementer 1424 s wall / $2.185 (89% of cost, 110 tool calls, 4 shell errors), tester gate 210 s (green first try), planner 128 s / $0.232, reviewer 24 s / $0.048. Recommendations ordered by expected impact:

1. **Warm the Rust build in the run container — seed fabro-c643 (open, `needs-user`).**
Evidence: implementer tool time 714.6 s + tester `just qualitygate` 210 s ≈ 15.4 of the 30.1 min wall was cold Rust compile/test in a fresh container (`ghcr.io/denkhaus/fabro-toolchain:99c855a689b3`, `lifecycle.preserve=false`).
Change: cargo-chef dependency layers in `.fabro/Dockerfile.toolchain`, or a persistent sccache volume via `run.environment` in `.fabro/workflows/develop/workflow.fabro`.
Effect: 4–7 min off every Rust-seed run — the largest wall-time lever visible in this run. (ADR-0019 capability-affecting; awaiting user approval per the seed.)

2. **Fix verify.nu's test detection for inline `#[cfg(test)]` modules — seed fabro-2f70 (open).**
Evidence: implementer journal painpoint this run — `scripts/verify.nu` classified fabro-server as code-touched only (compile check) although the seed added 6 tests inside `#[cfg(test)]` modules in `src/server/handler/sessions.rs`; the implementer had to notice and manually run the full 966-test crate suite on top of verify.
Change: `touched`/`is-test-file` in `scripts/verify.nu` (lines ~70–88) must also treat a src file whose diff hunks fall inside a `#[cfg(test)]` module as test-file-touched.
Effect: the one mechanical verify call matches policy again — no duplicate manual suites, and no class where test breakage sails through verify as a "compile check" and only surfaces at the tester (historically a ~22-min gate-red bounce).

3. **Backfill `pull_request.state` for terminal-but-unmerged runs — seed fabro-4bb7 (open).**
Evidence: planner observation this run — the in-flight guard ran with zero exclusions because PRs #221/#223 report `state: null` on succeeded runs; null "does not count as in-flight per the rules", so the check was blind exactly on the freshest runs where the claim-race window (fabro-22e4/fabro-91ff class) lives.
Change: `annotate_pull_request_states` / projection in `lib/components/fabro-tool/src/runs_list.rs` — resolve live state for terminal runs with a stored PR link instead of leaving null.
Effect: the double-pick guard stops depending on absence of data; when it matters, it actually excludes.

4. **Add the stale-test-binary guard to the implementer hard rules — new-seed justification: the class is recorded twice as expertise (mx-a74829/mx-c83b9d, root cause unresolved) but no seed adds the prompt rule, and it recurred in THIS run despite the lessons.**
Evidence: implementer observation — after fixing the failing assert, the first `cargo nextest run` re-run (1.2 s, no rebuild) still failed with the OLD panic text; a forced `touch` rebuild fixed it (one of the 4 shell-error/diagnostic beats).
Change: one line in `.fabro/workflows/develop/prompts/implementer.md` hard rules (next to the timeout rule): "a test failure that survives an obvious fix ⇒ force a rebuild (`touch <file>` / `cargo clean -p <crate>`) before re-diagnosing".
Effect: kills a recurring daily trap that costs a wasted re-run plus a wrong-root-cause risk each time it fires.

5. **Raise the reviewer's `preamble_inline_max_kb` from 16 to 32 — seed fabro-cf3e (open).**
Evidence: this run's evidence capture was 26,040 bytes > the 16 KB inline cap, so it blob-ref'd; the reviewer's response opens "Evidence blob read in full … page-by-page" and its journal confirms paging — a tool detour per review despite the 48 KB graph budget whose stated goal (fabro-1e9f) was zero blob detours for per-seed captures.
Change: reviewer node attr in `.fabro/workflows/develop/workflow.fabro`.
Effect: typical Rust-seed captures (~18–26 KB) render inline; review stays a context-first judgment (this one was 24 s / $0.048 — keep it that cheap).

6. **Deterministic stale-anchor check in planner-preflight — new-seed justification: no existing seed verifies anchor *content* (fabro-4c81 is path-existence only; fabro-c0ca/fabro-d20f govern brief-side quoting).**
Evidence: the planner burned ~42 s (18:01:43→18:02:20, incl. a 19-s reasoning turn) adjudicating fabro-53d3 — a High-priority seed whose line anchors ("planner exits are lines 127-129") rotted to 367–399 and whose target (a `refiner` node) never existed; preflight had marked it "clean" because it only greps for landed seed-id commits, not criteria-satisfied-by-symbol.
Change: `.fabro/workflows/develop/scripts/planner-preflight.nu` — for candidates whose body cites `file:line` anchors, check the cited line still contains the claimed content and flag mismatches in the verdict table.
Effect: dead seeds close in a sub-second script verdict instead of a planner reasoning lap, and stop occupying top priority slots for weeks (fabro-53d3 sat since 09-02).

7. **Top-N `sd ready` view instead of the firehose — seed fabro-c3b4 (open).**
Evidence: the planner's first tracker call returned all 200 ready issues, 28.8 KB of stdout (tool output, seq 38), of which only the top candidates mattered (preflight already narrows to 5).
Change: `sd ready` default/top-N form in the planner's command table (PROJECT_FACTS in the planner prompt / `docs/agents/issue-tracker.md`).
Effect: smaller planner tool outputs and preambles; modest here (planner = 9% of run cost) but pays on every pass.

Not recommended as changes (worked as designed this run): the preflight already-landed node, gate-bounce enrichment (never triggered — gate was green first try), and the planner's superseded-close of fabro-53d3 itself (correct hygiene, just mis-priced — see #6). One gap I could not fully inspect: the individual implementer shell-call sequence (only aggregates and 4 unexplained-error count are in the stage summary), so I did not rank tool-batching seeds like fabro-866a against evidence I couldn't verify.
