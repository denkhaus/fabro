# Improve review — run 01M1Z017EQK3Q2QCEMTA6XXNSK

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (4.1 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-07 22:38+0000 by revisor `fabro_ask`

---

All evidence below is from this run's events, journals, and workspace files (run 01M1Z017EQK3Q2QCEMTA6XXNSK, seed fabro-16ff, 208 s wall, $0.197 total: planner $0.056 / implementer $0.099 / reviewer $0.042; scripts 6 s total, zero retries, first-pass approval — so these are optimizations, not fire-fighting).

## Recommendations, by expected impact

**1. Close the fail-open in-flight PR guard — `.fabro/workflows/develop/prompts/planner.md:37` (step 4)**
What happened: the guard ran `gh pr list --state open 2>&1 | head -30`; `gh` is absent from the toolchain (reverted per ADR-0019, commit f00fdb1), the pipeline masked the failure (`head` exits 0, tool reported `is_error: false`, events seq 39–41), and the planner degraded to "no PRs" — its own journal painpoint confirms the double-pick guard was inert this run.
Change: replace the `gh` check with the credential-less `fabro_runs_list` filter already decided in open seed fabro-06e0 (filter runs with open linked PRs → extract seed ids from goals → skip + journal `skipped: in-flight PR <n>`).
Expected effect: the guard actually prevents duplicate work (fabro-22e4 precedent: an entire fix re-implemented while its PR waited); eliminates one guaranteed dead shell call every planner pass.

**2. Stop blob-ref'ing the evidence capture — `.fabro/workflows/develop/workflow.fabro:16` (`preamble_budget_kb=24` → 32, or exempt `command.output` from the aggregate demote)**
What happened: a 10.2 KB capture was demoted to a path-preview even though the reviewer's context window stood at 1.6 % of 1M and `preamble_inline_max_kb=16` (workflow.fabro:243) should have held it — the 24 KB aggregate (raised from 12 for exactly this reason, per the graph comment) was still exceeded by brief (2.2 KB) + implementation_summary (2.5 KB) + stage sections. The reviewer had to page `/tmp/fabro/runtime/blobs/8109…json` by hand (its journal painpoint; `read_file` was the one tool it invoked).
Expected effect: one fewer tool round-trip and ~10 s per review, and removes the "unread blob ref" class that feeds Verification-blocked retries. Matches open seed fabro-8d2c.

**3. Resolve the implementer prompt's self-contradiction on seed re-fetch — `.fabro/workflows/develop/prompts/implementer.md:22` (step 1)**
What happened: the Input section says "read the brief FIRST; fetch the full seed only if the brief is thin," but step 1 mandates "Re-read the seed requirements from `sd show`" — so the implementer re-fetched fabro-16ff (event seq 86) although the in-context brief was complete and verbatim from the seed body.
Change: make step 1 conditional: "`sd show` only when the brief is thin, ambiguous, or carries review feedback; otherwise the brief is the spec."
Expected effect: one fewer shell call + model round per implementer pass (~5 s + tokens); open seed fabro-4881 already proposes exactly this.

**4. Make the planner verify spec naming against the target files — `planner.md`, step 7**
What happened: the brief forwarded "checklist gains a CAPABILITY DELTA axis" verbatim, but `reviewer.md` has no heading named "checklist" (the numbered "Your job this pass" list is it). The implementer caught it and burned a ~14 s / 519-reasoning-token block deciding placement (events seq 95–97), then journaled it as an observation.
Change: extend step 7 (contradiction check) with: "when the spec names a heading/anchor/path, confirm it exists in the target file; when it doesn't, annotate the actual location in the brief."
Expected effect: shorter implementer passes and fewer misplacement deviations → fewer Changes-requested cycles (a cycle costs ~2.5 min + ~$0.15).

**5. Tell the reviewer the capture is self-terminating — `.fabro/workflows/develop/prompts/reviewer.md`, blob-ref paragraph**
What happened: `evidence.nu` already prints `== evidence complete ==` (`scripts/evidence.nu:431`) and the reviewer journaled that this marker "was the deciding signal" for completeness — but it only discovered the marker after paging to the end.
Change: one sentence in the blob-ref paragraph: "the capture ends with `== evidence complete ==`; seeing it means the diff was not cut."
Expected effect: reviewers approve from a single confident paged read instead of reading to EOF or flirting with Verification blocked (a re-capture + re-review is ~40 s).

**6. Make checkpoint metadata snapshots non-blocking — engine-side (`lib/`, open seed fabro-cf03)**
What happened: snapshots cost 2,090 ms at init and 1,999 ms at the planner checkpoint (events seq 16, 71), serial on a 208 s run — ~7 % of wall across checkpoints, purely bookkeeping.
Expected effect: ~10–15 s saved per run, proportionally more on multi-cycle runs.

**7. Fix the "Pipeline progress" numerator — engine header (open seeds fabro-a0e3/45bf)**
What happened: the implementer's prompt said "0 of 7 stages completed" after the planner had completed (22:33:49 vs 22:33:43); the reviewer's said "2 of 7" with four stages done.
Expected effect: truthful progress for agents and the web UI; trivial change, cosmetic impact.

**8. (Minor) Downgrade the benign allow-key warning** — the run's only WARN line is `preamble_allow_keys entry absent … key=output.gate_known_bug_hits` on the implementer, an expected condition on a green first visit (open seed fabro-8275 wants it at info). Effect: cleaner logs, real warnings stay visible.

Not worth changing based on this run: the tester gate (5.3 s, correctly self-scoped to "no crates touched" for a prompt-only seed — fabro-01b9's path-scoping would save nothing here), and tool batching (planner and implementer already batched independent calls in single blocks; the fs_hide shell carve-out worked as documented).
