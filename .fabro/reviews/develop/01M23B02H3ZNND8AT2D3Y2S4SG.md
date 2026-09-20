# Improve review — run 01M23B02H3ZNND8AT2D3Y2S4SG

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (19.3 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-09 15:25+0000 by revisor `fabro_ask`

---

# Recommendations for run 01M23B02H3ZNND8AT2D3Y2S4SG (seed fabro-4814, "purge two-worlds framing")

**Run profile (from run events / conclusion):** 19m18s wall, $1.207 total, zero retries, all 7 stages first-pass green, PR #97. Implementer: 902.7s inference / $1.035 (**86% of run cost, 84% of wall**), 13.2s tool time. Planner $0.062, Reviewer $0.110, tester 4.8s, evidence 0.36s, closeout 0.30s.

---

**1. Fix the fs_hide collision for loop-asset-targeting seeds — the run's dominant friction.**
*Evidence:* event seq 95 — implementer's first `read_file` on `.fabro/workflows/develop/prompts/implementer.md` returned "hidden… behaves as if it did not exist"; it then did **every** read/edit of the seed's 10 target files through shell heredocs (python3 replace scripts with exact-match asserts), and journaled exactly this as its painpoint. The seed's entire scope was `.fabro/**`, so the "carve-out" (shell workaround) was the *main* path, not an exception, and contributed to the 15-minute implementer.
*Change:* engine-side, a per-seed fs_hide exception when the claimed brief's paths are all inside the fs_hide list (new seed; the stage-envelope code is the `fs_hide` resolution in `lib/components/fabro-workflow`). Cheap interim: land open seed fabro-d02d — make the read_file denial message name the shell bypass (today it reads like a bug, costing one wasted call + reasoning turn per agent, every loop-asset seed).
*Effect:* eliminates the discovery turn and the error-prone heredoc-edit mode for every workflow-asset seed; directly attacks the 86%-of-cost stage.

**2. Kill the reviewer evidence-blob detour — either raise the budget or cap the loop-diff.**
*Evidence:* evidence capture was **55,353 bytes** (script_timing, evidence@1) vs. the reviewer's `preamble_inline_max_kb=16`; the reviewer had to page it with `nu str substring` **in three calls** before judging (reviewer journal painpoint + response "paged in 3 chunks"), inside a context window at **3.1% of 1M** — the 24KB budget, not the window, forced the detour. This is a loop-work seed, where the loop-churn diff *is* the review scope, so the capture ballooned past what the 16KB inline cap was sized for ("captures reach ~14KB").
*Change:* in `workflow.fabro` — graph `preamble_budget_kb` 24→32 and reviewer `preamble_inline_max_kb` 16→32 (open seeds fabro-8d2c / fabro-cf3e already propose this; this run is fresh proof), **or** add a per-file hunk budget for the loop-work diff section in `scripts/evidence.nu`.
*Effect:* one read instead of three per review; removes substring-paging misparse risk and ~60–90s of reviewer latency on every asset-heavy seed.

**3. Conditionally scope the implementer's Rust-gate rules — they were dead weight on this config-only seed.**
*Evidence:* implementer consumed 78.4k input tokens / 13.9k reasoning tokens; step 4 of `implementer.md` carries ~1.5KB of clippy/nextest/default-features rules plus five run-id citations, none of which applied (no Rust touched; the gate printed "no crates touched … format clean"). Open seed fabro-7b2a already names this.
*Change:* in `.fabro/workflows/develop/prompts/implementer.md`, keep step 4 to the cost-tier rule ("config-only → parse-level check") and move the crate-scoped clippy/nextest block under an explicit "only when the seed touches Rust" sub-heading (or into `project-facts.md`).
*Effect:* smaller, sharper prompts for every config-only seed; less rule-dilution for the Rust case that actually needs them.

**4. Trim the planner's firehose inputs.**
*Evidence:* seq 32 — `sd ready --assignee fabro --limit 200` returned **189 issues / 25,613 bytes** to pick one candidate; seq 41 — `fabro_runs_list` returned **12,676 bytes / 14 runs** where only `status` + `pull_request.state` + `goal` matter for the in-flight guard.
*Change:* planner.md command table — switch to a top-N view (`--limit 10`, open seed fabro-c3b4); pass `created_since` to `fabro_runs_list` (in-flight runs are recent by definition).
*Effect:* ~35KB less context in the planner's early turns; modest dollar impact (planner is cheap) but faster first-output and less distraction for the tie-break.

**5. Keep concrete literals at the new PROJECT_FACTS indirection sites.**
*Evidence:* reviewer observation (non-blocking): planner.md step 4 now says "shell grep for the seed id prefix (PROJECT_FACTS)" where `fabro-` used to be inline — an LLM must now resolve the indirection through the include to run the double-pick guard correctly.
*Change:* in `prompts/planner.md` (step 4) and `prompts/project-facts.md`, put the concrete example at each use site ("the seed id prefix (e.g. `fabro-`)").
*Effect:* removes a mis-parse class from the exact mechanism that prevents fabro-22e4-style double-picks — near-zero cost insurance.

**6. Add a long-stage heartbeat notification.**
*Evidence:* Slack notifications are configured only for `run.completed`/`run.failed` (run settings); the implementer ran **15 of the run's 19 minutes** with no external signal — an operator had no mid-run cue that a single stage dominated, or a hook to cancel.
*Change:* in `workflow.toml` notifications block, add a stage-level event (or a "stage exceeds N minutes" heartbeat) alongside the terminal events.
*Effect:* visibility into exactly the pattern this run exhibited (single-stage cost concentration) while intervention is still possible.

---

**What worked — don't touch:** the deterministic gatebounce/closeout pattern (closeout: 0.30s, closed exactly fabro-4814), the mandatory journal contract (all three agent stages emitted both keys, first run-visible painpoints landed within minutes of the friction), the gate's cheap path for config-only seeds (4.8s), and the MiniJinja `{% include %}` PROJECT_FACTS mechanism (mx-bf9c40) — the run proved it end-to-end by rendering its own new prompts through the include at the reviewer stage.
