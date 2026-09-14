# Improve review — run 01M2GVW7GGGB19ZZ9NBN2H5FDC

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (34.3 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-14 22:04+0000 by revisor `fabro_ask`

---

## Run economics (grounding)

From run events/checkpoints: 33.6 min active, $0.728 LLM (planner $0.06 / implementer $0.55 / reviewer $0.12), one seed (fabro-b4ed) shipped via PR #142. The run contained **one avoidable gate-red cycle**: tester@1 red (247 s) → gatebounce → implementer@2 ($0.186, 286 s) → tester@2 green (242 s) — roughly **9–13 min of wall time and 26% of run cost**. Everything below traces to observed stage transcripts, journals, and timings.

## Recommendations, by expected impact

**1. Fix the broken `just verify implementer` script — `scripts/verify.nu:41`.**
What happened: implementer@1's journal painpoint (mx-e606d5): `parse --regex` with an interpolated `$'…'` string parses `(?P<sha>…)` as a nushell subexpression on nu 0.115.0, so **the one authorized mechanical verification call never ran**; the implementer fell back to manual per-crate judgment — and skipped the fabro-server suite (the salvage *did* touch `server/tests.rs`, so fabro-server was a test-file-touched crate; the gate's own header later listed all 7 touched crates). The stale fixture in `run_tool_create.rs` then red-lined the gate (1 of 4699 tests).
Change: one line — plain single-quoted regex string in `scripts/verify.nu`. It's a loop asset (report-don't-fix applied correctly), so it needs a seed or direct user fix.
Effect: mechanical touched-crate derivation returns; the fabro-server suite runs pre-gate and this drift class surfaces ~4 min into implementation instead of at the gate — eliminating the entire bounce cycle (~9–13 min, ~$0.19 per affected run).

**2. Add downstream-caller focused tests to the verification policy — `.fabro/workflows/develop/prompts/implementer.md` step 4 (or extend verify.nu).**
What happened: implementer@2's journal painpoint: crate-scoped verify couldn't see cross-crate fixture drift; the first gate was its only signal (~557 KB log). Defense-in-depth for the case where the downstream crate is *not* in the touched set.
Change: one sentence — "when a seed changes a cross-crate contract (e.g. `inherit_parent_target`), `rg` the workspace for callers/tests of the changed symbol and run those focused tests too" (the actual retest cost 0.37 s).
Effect: catches fixture drift even when verify's crate scope misses it; turns a 4-minute gate-red diagnosis into a seconds-long focused failure.

**3. Tighten gate-bounce matching to error signatures — `.fabro/workflows/develop/scripts/gate-bounce.nu`.**
What happened: on the red bounce, gatebounce injected 3 hits (fabro-f18a prompt-JSON lint, fabro-6a78 preamble fidelity, fabro-71b9 kanban tiles) — **none related** to the actual failure (a Rust fixture drift). ~3 KB of irrelevant seed bodies went inline into implementer@2's prompt.
Change: match failure tails against error signatures (failing test names, error classes) instead of loose keyword tails; prefer empty hits over noise (open seed fabro-c841 wants exactly this; this run is direct evidence).
Effect: bounce prompts start at the true root cause (here: the named failing test) with no noise; removes misdiagnosis risk on every red bounce.

**4. Require English output — `.fabro/workflows/develop/prompts/implementer.md`, Output hygiene section.**
What happened: implementer@1's entire `implementation_summary` and response were German ("Zusammenfassung: Die Strandung aus …"), and that text flowed verbatim into the reviewer's `## Current context` and the checkpoints. Open seed fabro-eaa0 exists; this run is fresh evidence.
Change: one line — "Write `implementation_summary`, journal entries, and all emitted text in English."
Effect: uniform downstream parsing and review; zero cost.

**5. Emit a per-file diff-stat map at the top of the evidence capture — `.fabro/workflows/develop/scripts/evidence.nu`.**
What happened: reviewer@1's journal painpoint: the 55 KB capture arrived as a blob ref that needed **three `nu str substring` paging reads** to reassemble, plus `python3 -c` prints silently produced empty stdout in the reviewer sandbox, costing extra verification turns before approval.
Change: put a compact per-file +/− map in the header (ahead of the full diff), as the reviewer itself suggested; this 55 KB data point also supports open seeds fabro-8d2c/fabro-cf3e (budget/inline raises).
Effect: the reviewer confirms capture completeness from the inline preview without the blob round-trip — fewer tool turns and less verification-uncertainty risk per review.

**6. Make salvage pointers name the diff command, not just the branch tip — seed-filing/planner brief guidance (`.fabro/workflows/develop/prompts/planner.md` step 6).**
What happened: salvage-first demonstrably worked (~1 hour saved; stranded commit da3eaabb3 needed only 1 conflict resolution) — but the implementer first diffed the stranded *tip vs HEAD* (228 files of divergence, one wasted call) before realizing the correct base was `git show da3eaabb3`.
Change: when a seed cites a stranded branch, cite the implementer commit and the `git show <commit>` form.
Effect: removes the mis-based diff call on every salvage pass (common after 429/park failures).

**7. Stop zero-signal focused test runs — `implementer.md` step 4.**
What happened: implementer@1 self-reported two policy-exceeding focused runs (`run_wait`: 307 s / 0 hits, `pr_view`) ≈ 5 min of tool time with no signal — while the one suite that mattered (fabro-server) went unrun.
Change: "any focused test beyond the derived touched-crate classes must name the acceptance bullet it serves."
Effect: ~5 min saved per implementer pass, redirected to tests that actually catch drift.

**8. Trim the `sd ready` firehose — PROJECT_FACTS command table in `planner.md`.**
What happened: the planner's first call returned 200 seeds / 28,021 bytes, stdout-truncated; only the top High-priority rows were ever used.
Change: pipe the listing through `head -40` (sd ready is priority-ordered) while keeping `--limit 200` semantics; open seed fabro-c3b4 wants the same.
Effect: smaller planner context and no truncation of the rows that matter; small but free.

**9. Engine-side dedup of repeated command-stage sections in preambles (engine, noted in `workflow.fabro` graph comments).**
What happened: reviewer@1's prompt carried the *green tester output twice* plus the gatebounce block — per-visit stage sections accumulate even when byte-identical.
Effect: modest preamble shrinkage per review cycle; the graph comment already flags this as engine dedup work — this run confirms it's still live.

Not inspected: the tester/gate scripts' internals beyond their captured output, and the blobs' full contents (only previews were readable); recommendations 1–3 rest on the implementer/reviewer journals and the gate output captured in events.
