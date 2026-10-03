# Improve review — run 01M2MWB09SZGD68YSBHYJFH4XP

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (2.4 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-16 10:39+0000 by revisor `fabro_ask`

---

All findings below are grounded in this run's events, checkpoints, stage timings, and per-turn LLM usage (run `01M2MWB09SZGD68YSBHYJFH4XP`, seed `fabro-50c9`, total 136 s wall / $0.164: planner $0.082 = 50%, implementer $0.054 = 33%, reviewer $0.028 = 17%; zero retries, zero failures, PR #173). Recommendations ordered by expected impact; every one names an existing open seed from the tracker listing captured in this run's `sd ready` output.

**1. Collapse the planner's reconnaissance round-trips into one batched shell call — seeds `fabro-55a7` + `fabro-66bc`.**
Evidence: the planner made 6 sequential tool calls (`sd ready --limit 200` → `fabro_runs_list` → `sd show` → grep of `reviewer.md` → `git log --grep` → claim), each followed by a 6–10 s LLM turn; tool time was only 3.4 s of its 57 s stage. The `sd ready` call returned 27.9 KB / 200 seeds when the planner picked the first line (`fabro-50c9`). Change: in `.fabro/workflows/develop/prompts/planner.md` steps 1–5, prescribe ONE shell call batching top-N `sd ready` + `sd show <candidate>` + `git log --grep <id>` (fabro-66bc's top-N invocation, fabro-55a7's batch). Expected effect: ~7 LLM turns → ~3–4, saving roughly 20 s and ~$0.03 per run (~20% of run cost).

**2. Split PROJECT_FACTS per role so the implementer/reviewer stop carrying planner-only payload — seed `fabro-52b4` (companions `fabro-7b2a`, `fabro-46a4`).**
Evidence: the implementer's first LLM turn read 18.7 k input tokens ($0.026, 48% of its stage cost before any tool) for a job that was a one-line markdown insert; its prompt carries the full `sd` claim/close/update table it is forbidden to use plus the entire step-4 Rust clippy/nextest policy. The reviewer read 18.1 k input with `cache_read: 0` (all billed fresh), including that same sd table ("reviewer none" is exactly fabro-52b4's scope). Change: `.fabro/workflows/develop/prompts/implementer.md` + `reviewer.md` via the project-facts include — planner keeps the full table, implementer keeps only `sd show`, reviewer drops it. Expected effect: ~$0.02–0.03 per run (~15%) and fewer rule-collision surfaces in prompts that already exceed 200 lines.

**3. Fix the pipeline-progress counter (both surfaces) — seed `fabro-45bf`.**
Evidence (fresh reproduction in this run): the implementer's stage prompt read "Pipeline progress: 0 of 7 stages completed" after start+planner were done; the reviewer's read "2 of 7" with 4 non-meta stages completed; the final projection says "6 of 7". Exactly the cumulative-vs-unique bug fabro-45bf documents (run-level projection AND stage-preamble header). Change: engine progress computation, unique completed non-meta nodes / total, per the seed's scope note. Expected effect: honest mid-run progress for users watching the run and for any agent reasoning from the header; this run should be added to the seed's evidence list.

**4. Give the evidence and closeout nodes their own output keys — seed `fabro-9ef9`.**
Evidence: `command.output` was overwritten three times — checkpoint seq 119 holds the tester gate log (blob `ae010c7…`), seq 127 the evidence capture (`b484f31…`), seq 155 the closeout output (`5cf7a2f…`). Benign here only because the gate was green; on a red bounce the gate result would be silently clobbered. Change: key command-node outputs per node in `.fabro/workflows/develop/workflow.fabro` (+ `evidence.nu`). Expected effect: the gate log survives in context for the red-bounce path — pure error-handling robustness, zero cost.

**5. Make the in-flight check mechanical and bounded — seeds `fabro-9372` + `fabro-6b58`.**
Evidence: the planner's `fabro_runs_list` call returned 15.3 KB / 17 runs reaching back to 2026-09-14; every goal was the identical generic string with no seed id, so the planner manually reasoned "only my own run is non-terminal" (journaled as such). The tool supports `created_since` but the planner prompt doesn't direct it. Change: planner.md step 4 — pass `created_since` (~48 h) and self-exclude by run id (fabro-6b58), and land current_seed_id in the projection (fabro-9372, engine-side). Expected effect: the mid-flight claim guard becomes a mechanical filter instead of a 15 KB context blob plus per-run reasoning.

**6. Make small-churn run outputs self-explaining — seeds `fabro-1409` + `fabro-b1d3`.**
Evidence: PR #173's diff is 9 additions across 3 files of which exactly 1 line (`reviewer.md`) is the real change; the rest is journal + tracker bookkeeping, and the Slack `run.completed` notification carries no weight signal. Change: PR postlude appends the per-stage cost/time table + journal digest (fabro-1409); Slack payload annotates near-bookkeeping runs (fabro-b1d3). Expected effect: a human reviewing PR #173-style PRs sees what ran, what it cost ($0.16, 2.5 min), and what the loop observed, without opening the run.

What worked — keep as-is: the deterministic tester (3.6 s, correctly skipped Rust for a prompt-only seed), evidence inline at 3.5 KB (no blob detour; the 48 KB budget did its job), and the reviewer's zero-tool 20.5 s approval, which beats fabro-50c9's own 35–40 s prediction — the clause this run added governed its own review pass successfully.
