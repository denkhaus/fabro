# Improve review — run 01M2MGAQ4TRWHGEY34P17K2R4D

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (5.4 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-16 07:11+0000 by revisor `fabro_ask`

---

All evidence gathered — stage transcripts (events seq 18–231), gate output, journals, worker log, and the tracker (`.seeds/issues.jsonl`) for seed coverage. Run shape for context: **5m22s wall, $0.492 total**; implementer = 177.7s / $0.351 (71% of cost, 65% of inference) for a **2-file, +6/−1 markdown-only seed**; planner = 66.5s / $0.095 over 9 LLM rounds; reviewer = 32.2s / $0.046; tester/evidence/closeout < 4s each. Zero retries, zero failures — so every recommendation below targets cost/latency/risk, not repair.

## Recommendations (by expected impact)

**1. Shrink and cost-tier the implementer's step 4 — seed fabro-7b2a**
What happened: the implementer burned 175.3s inference / 12 LLM rounds on a markdown-only seed whose step-4 policy block (fmt/clippy/nextest/cross-crate rules, ~2.5k chars) and full `sd` command table were 100% irrelevant; tool time was only 2.1s (from run events, stage timings). The seed's own reasoning trace (seq 95) shows a wasted meta-confusion round about its own stale prompt text.
Change: in `.fabro/workflows/develop/prompts/implementer.md`, cap step 4 to the operative rule + a one-line non-Rust fast path ("no Rust touched → parse/grep-level checks only"), move run citations to footnotes — exactly fabro-7b2a's demand (it also requires fixing the known tests-rule contradiction in the mechanical-rewrite clause).
Effect: prompt-edit seeds (most of this backlog) stop paying the Rust-policy reading tax; est. −30–50% of implementer inference on this seed class.

**2. Batch the planner's claim reconnaissance into one shell call — seed fabro-55a7**
What happened: the planner ran `sd ready` → `sd show` → `git log --grep` → two anchor greps → `fabro_runs_list` → probe-verification grep as **six separate round trips** (seq 29–60), each costing a 5–8s LLM turn — 66.5s / $0.095 for claim bookkeeping.
Change: `.fabro/workflows/develop/prompts/planner.md` steps 3–4: one combined probe shell (`sd ready` + `sd show <id>` + base-branch grep + anchor greps) before the claim.
Effect: ~4–5 fewer LLM rounds, est. −30–40s wall and −25–35% planner cost per run (fabro-2be2 is the duplicate-adjacent sibling; land one of them).

**3. Make the evidence capture arrive inline for the reviewer — seeds fabro-8d2c + fabro-cf3e**
What happened: the 11.4KB capture was demoted to a blob ref even though it's under the reviewer's 16KB per-key cap — the aggregate 24KB graph budget was exceeded — forcing one `read_file` detour; the reviewer journaled this as its only painpoint (from stage journal, reviewer@1).
Change: `.fabro/workflows/develop/workflow.fabro`: graph attr `preamble_budget_kb` 24→32 (fabro-8d2c) and reviewer attr `preamble_inline_max_kb` 16→32 (fabro-cf3e); fabro-e4c4's output_schema delivery is the durable follow-up.
Effect: one tool round-trip saved per review and the unread-blob → `Verification blocked` rejection class disappears (context window stood at 2% — the cap, not the window, is the constraint).

**4. Deterministic filed-vs-implemented match in the duplicate-run preflight — NEW seed**
What happened: the implementer's own preflight grep on `origin/denkhaus` hit `bd41cbac` — the revisor's seed-*filing* commit — so the landed fix relies on an LLM judgment clause ("only a merged IMPLEMENTATION is a duplicate") that every future implementer must re-derive correctly; recorded as mx-9aec69 (from implementer journal/summary).
New-seed justification: no open seed makes the *matching* deterministic — fabro-6b58/fabro-9372 are claim-race/projection engine seeds, and 58cb/7280 closed on the branch fix itself; mx-9aec69 explicitly says future fixes must encode this distinction.
Change: `.fabro/workflows/develop/prompts/implementer.md` step-1 preflight: grep only merge commits (`git log --merges --oneline origin/<merge-target> --grep "<seed>"` or match the `(#<n>)` squash-subject pattern).
Effect: the false-`Blocked` class on every revisor-filed seed stops depending on model judgment.

**5. Stop `sd ready --limit 200` pouring the whole tracker into the planner — seed fabro-66bc**
What happened: the planner's first call returned 200 seeds / 27,805 bytes, stdout-truncated (event seq 30), and it picked the first line anyway.
Change: planner.md command table → top-N priority-sorted `sd ready` invocation.
Effect: smaller planner context every run — cheaper turns, no truncation noise.

**6. Pre-create the `develop` ml domain so `ml record` stops mutating tracked config — seed fabro-b94d**
What happened: this run's diff shows `.mulch/mulch.config.yaml +develop: {}` — the implementer's lesson capture auto-created the domain mid-run, adding tracked-config churn the evidence capture classified as loop-churn (5 files instead of 4; from the run diff).
Change: pre-create the domain in the toolchain image (`ghcr.io/denkhaus/fabro-toolchain`), per fabro-b94d.
Effect: no surprise tracked-config churn per lesson capture; one fewer anomaly for the reviewer to adjudicate.

**7. Backfill `pull_request.state` for terminal-but-unmerged runs — seed fabro-4bb7**
What happened: the planner's in-flight guard saw PR #159 as `state: null` and ran in its documented degraded no-exclusions mode (planner journal observation, seq 54/75); had #159 actually been open, its seed was double-pickable — the exact claim-race family this run's seed patches prompt-side.
Change: engine `fabro_runs_list` projection (store/projection code): populate `pull_request.state` for terminal runs whose PR is unmerged.
Effect: the in-flight guard stops running blind precisely when a PR sits unmerged; composes with fabro-9372 (`current_seed_id` in the same projection, which would also have saved this planner its goal-parsing work).

**8. Forbid the implementer from introspecting its own run — NEW seed**
What happened: mid-pass the implementer called `fabro_run_get` on its **own** run (event seq 99), pulling the entire run projection — full graph with every node's prompt — into a context that already totaled 82k input tokens; the brief and PROJECT_FACTS already carried everything it needed.
New-seed justification: no tracker seed covers implementer-side run introspection (fabro-a08a is planner-side dead-context reads; fabro-a67f/fabro-4881 cover only the `sd show` re-fetch).
Change: one line in implementer.md's Input section: "never call `fabro_run_get`/`fabro_runs_list` on your own run."
Effect: avoids a multi-KB context dump and a likely extra reasoning round per implementer pass.

**9. Small hygiene, real but cheap — seeds fabro-8275 (+ one NEW)**
(a) The worker log warns `preamble_allow_keys entry absent … output.gate_known_bug_hits` on every green first visit — by design; fabro-8275 downgrades it to info. (b) Every agent session logs 2 ERRORs for missing `.codex/instructions.md` (6 total this run, from worker log) — *new-seed justification: no seed covers suppressing the codex-instructions probe in repos without that file*; fix the session init to probe-then-read. Effect: worker logs stop crying wolf, so genuine warn/error lines (like this run's PR-body JSON retry at 07:06:49, which the fabro-cd27 retry absorbed correctly) stay visible.

**One thing that worked and needs no change:** the gate (`just qualitygate`, 3.7s, "no crates touched"), `just verify implementer` (correctly derived "no lib/ crates touched"), the deterministic closeout (0.24s), and the planner's pre-claim seed-body correction (`sd update` recording the moot planner-probe sub-item before claiming) all behaved exactly as designed — from run events, gate output, and the tracker diff.
