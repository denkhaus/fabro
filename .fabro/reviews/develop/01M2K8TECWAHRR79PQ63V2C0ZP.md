# Improve review — run 01M2K8TECWAHRR79PQ63V2C0ZP

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (11.8 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-15 20:03+0000 by revisor `fabro_ask`

---

Run recap (from run events + journal): all 7 stages green first-pass, 11.5 min wall, $0.291 LLM cost (planner $0.114 / implementer $0.140 / reviewer $0.036), seed fabro-a03e closed, PR #157. The friction below is what the transcripts and journal actually show; every seed id was verified present in this run's own `sd ready` output (planner shell call, seq 30–31).

## Recommendations, by expected impact

1. **Fix verify.nu's blind spot for inline `#[cfg(test)]` tests — seed fabro-2f70 (open, @fabro).**
   What happened: the implementer's journal observation records that `just verify implementer` classified the `git.rs` edit as code-touched only, so verify degraded to a compile check and the new retry-jump test ran only at the deterministic gate. This is the *second* occurrence of fabro-2f70's exact case, same file (`src/lifecycle/git.rs`), one run after it was filed.
   Change: `scripts/verify.nu` `is-test-file` (line 24) — also flag a crate test-file-touched when the diff touches `#[cfg(test)]` regions.
   Effect: the pre-gate test signal fires on every inline-test seed (the project convention); a broken new test fails in the implementer's own pass instead of burning a gate-red bounce cycle.

2. **Stop evidence.nu flagging seed-spec-named loop assets as anomalies — seed fabro-93a7 (open).**
   What happened: the reviewer's journal painpoint — the capture listed `.fabro/workflows/develop/prompts/implementer.md` under "changed files NOT named by the seed spec" although fabro-a03e names it explicitly; the reviewer had to adjudicate the false anomaly manually before approving.
   Change: `.fabro/workflows/develop/scripts/evidence.nu` — diff the loop-path set against paths cited in the seed spec before emitting the anomaly section.
   Effect: mixed loop-asset seeds stop generating false residue alarms; removes a plausible false "Changes requested" cycle on every spec-compliant loop-asset seed.

3. **Backfill `pull_request.state` in the runs-list projection — seed fabro-4bb7 (open).**
   What happened: the planner's in-flight guard ran in its documented degraded mode — run 01M2K73FA's PR #155 showed `state: null`, so "no exclusions" applied (planner journal observation, seq 33 tool result). Had #155 been open, its seed was double-pickable.
   Change: engine `fabro_runs_list` projection (store/projection code) — populate `pull_request.state` for terminal-but-unmerged runs.
   Effect: the graph's duplicate-claim guard (fabro-06e0/fabro-91ff edge case) stops running blind exactly when an unmerged PR sits in the gate; prevents a duplicated ~12 min / $0.29 run.

4. **Batch planner reconnaissance into one shell call — seed fabro-55a7 (open).**
   What happened: the planner ran six ~50 ms micro-probes (seq 43, 49, 55, 61, 67, 73: git log grep, sed ranges, signature greps), each followed by a 5–8 s LLM round trip — 9 LLM turns total, 76.9 s wall / $0.114 = 39% of run cost for claim bookkeeping.
   Change: `.fabro/workflows/develop/prompts/planner.md` steps 3–4 — prescribe one combined probe shell (anchors + cases + git-log grep) before the claim.
   Effect: ~5 fewer LLM round trips per run; est. −30–40 s wall and −25–35% planner cost.

5. **Raise preamble budget 24 → 32 KB — seed fabro-8d2c (open).**
   What happened: the reviewer's prompt carried the evidence capture as a blob ref — `Output (7.9 KB; full value: /tmp/sandbox-driver/runtime/blobs/c47ab3a1….json)` — despite `preamble_inline_max_kb=16` on the node, forcing one `read_file` detour before judging.
   Change: `workflow.fabro` graph attribute `preamble_budget_kb` (graph attrs block).
   Effect: sub-16 KB captures render inline; the reviewer decides from context with no blob paging — the exact detour 8d2c was filed for, now reproduced in this run.

6. **Use a top-N `sd ready` view — seed fabro-c3b4 (open).**
   What happened: `sd ready --assignee fabro --limit 200` returned 200 seeds / 27,693 bytes into the planner's context (event seq 30, stdout truncated); the planner picked the first line (fabro-a03e, High).
   Change: planner prompt command table — top-N (priority-sorted) `sd ready` invocation.
   Effect: smaller planner context on every run; cheaper turns, less truncation noise.

7. **De-duplicate the hand fmt/clippy pass against `just verify implementer` — new seed.**
   What happened: the implementation_summary shows the implementer ran focused nextest + hand fmt + hand clippy *and then* `just verify implementer`, which re-ran the same pinned fmt+clippy per code-touched crate — a double compile pass inside the implementer's 429.9 s tool time, mandated by both the brief and implementer.md step 4.
   New-seed justification: no open seed covers this duplication — fabro-a67f whitelists smoke checks, fabro-a643 names acceptance bullets for focused tests, fabro-7f58 narrows test scope; none removes the brief/prompt's fmt+clippy triplet that verify already mechanizes.
   Change: `.fabro/workflows/develop/prompts/implementer.md` step 4 and `planner.md` step 7 — briefs prescribe `just verify implementer` plus only genuine supplements (focused nextest until fabro-2f70 lands, feature-scoped clippy, cross-crate caller tests).
   Effect: one fewer compile pass per implementer stage; shorter briefs; removes the "verify vs prose" conflict surface.

8. **Add per-stage cost/time table + journal digest to the PR body — seed fabro-1409 (open, UX).**
   What happened: this run's PR #157 ships the diff, while the two genuinely useful findings (verify.nu downgrade, evidence.nu false anomaly) live only in the run-branch journal.
   Change: PR postlude (engine postlude/PR-body generation) per fabro-1409.
   Effect: a user reading the PR sees cost, timings, and friction without opening the run.

Not actionable from this run: fabro-581b (mold in the run image) — labeled `needs-user`, and the user already deferred it ("später"); no error-retry behavior was exercised (0 retries, gate green first try), so retry-policy changes would be ungrounded here.
