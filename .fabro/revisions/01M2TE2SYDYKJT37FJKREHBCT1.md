# Revision — run 01M2TE2SYDYKJT37FJKREHBCT1

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2TE2SYDYKJT37FJKREHBCT1.md
- seeds filed: fabro-679a (Count direct single-parent base-branch fix commits as landed in dup-run-check.nu); fabro-8678 (Forbid compile-level probes as Already-landed closure evidence in the planner prompt)
- basis: run 01M2TE2SYDYKJT37FJKREHBCT1, workflow version 8cb9aa2b3f9ba4ff5230dfcb80653ace3d31b103c847903e87451a06a36f8038, commit 86d4a2c5029f27fc6d41f160bd64420ce2d00bc9
- revised_at_commit: 86d4a2c5029f27fc6d41f160bd64420ce2d00bc9 (ADR-0015: engine drift signal for later judgement)

## Findings

### Count direct single-parent base-branch fix commits as landed in dup-run-check.nu
- filed: fabro-679a
- In `.fabro/scripts/dup-run-check.nu`, `landed` counts 2-parent merges plus PR-suffixed no-merge commits only; a direct fix commit referencing the seed id without a `(#n)` suffix falls into `$other`, so the preflight returned `clean` and the planner burned 15 tool calls / 559 s / $0.263 re-deriving a close the script exists to do (closed fabro-a32f) — the fix commit was the run's own base SHA. Change: fold seed-id-referencing single-parent base-branch commits into `landed`, routed through classify-filed so filing-only shapes (fabro-ead4 evidence) are not miscounted. Expected effect: this run class (~9.6 min, $0.26 each) collapses to a ~30 s zero-LLM preflight exit. No duplicate found: fabro-ead4 targets planner-preflight.nu superseded-close; closed fabro-0d48/fabro-8c75 are false-positive narrowing; fabro-a01f is the claim race.

### Forbid compile-level probes as Already-landed closure evidence in the planner prompt
- filed: fabro-8678
- In `.fabro/workflows/develop/prompts/planner.md`, the ALREADY-LANDED arm (line 51) sets no cost tier for closure evidence, so the planner ran a cold `cargo nextest run -p fabro-workflow routing_contract` — 290,172 ms wall (50.4% of run wall) for a test that ran in 0.011 s. Change: one sentence in that arm — closure evidence is commit diff inspection (`git show`) plus the landed commit's own test suite; compile-level probes forbidden on this route. Expected effect: removes ~290 s before any cache warming; complements open fabro-c643 rather than depending on it. No duplicate found: closed fabro-6ae9's cost-tier rule binds briefs only, fabro-4be6 is brief dry-runs, fabro-7f58 implementer test scoping.
