# Improve review — run 01M0QJ01P9E6EKX8S6TQN91G8E

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: failed (17 sec, revisor pass — reason and cost in run detail)
- generated: 2026-09-04 22:10+0000 by revisor `fabro_ask`

---

All recommendations below are grounded in this run's events, worker log, and workspace files. Context: the run did **zero work** — 0 tokens, 0 files changed (from run events usage/diff summaries) — yet cost 50.7s of wall time (created 14:57:03 → failed 14:57:53) because the very first LLM call was rejected.

## What actually happened (evidence baseline)

- 14:57:03–14:57:36 (33.3s): sandbox boot 3.1s, clone 2.1s, `mise install` **26.6s**, `just bootstrap` 1.8s (from worker log).
- 14:57:38: planner (agent, glm-5.3, `reasoning_effort=medium`) → first API call rejected in 1.5s: *"model 'glm-5.3' does not support reasoning_effort 'medium'; allowed values: low, high, max"* (event seq 37/39; failure class `deterministic`, `will_retry=false`).
- Routed planner→refiner→exit; two empty-diff checkpoint commits+pushes (dfb266e, d99a357; each ≈1.7s snapshot + ≈2.5s push) consumed ~8.5s of the 17.5s active window. Terminal reason: *"goal gate unsatisfied for node planner and no retry target"* (event seq 58).

## Recommendations, by expected impact

**1. Fix the root cause: `reasoning_effort="medium"` is invalid for the configured model.**
- Change: `.fabro/workflows/develop/workflow.fabro:29` (planner node) — `reasoning_effort="medium"` → `"low"` (planning is a distill task; low fits) or `"high"`.
- Effect: the planner's first call succeeds; today this single line deterministically kills 100% of runs at visit 1. Everything else below is secondary.

**2. Validate model-capability attrs at submission, before sandbox+setup.**
- Change: engine submit path (`fabro_workflow` validation) — check node attrs (`reasoning_effort`, `speed`) against a per-model capability table (the provider even returns the allowed list in its error) at `run.start_requested`.
- Effect: this error class fails in <1s instead of after 33s of sandbox/clone/mise/bootstrap that produced nothing; no infra spend on guaranteed-failure configs.

**3. Bake the toolchain into the runner image (or cache mise installs).**
- Change: build `fabro-runner:mise` with the `.mise.toml` toolset pre-installed — the file itself (lines 1–2, 18–19) acknowledges this: "lean base, no baked-in tools… restore when the on-demand image build exists".
- Effect: removes 26.6s (52% of this run's total lifetime) from **every** run, and eliminates the aqua/Sigstore network fragility that already forced lefthook/betterleaks out of the quality gate (`.mise.toml` lines 10–19).

**4. Surface the root cause in the terminal failure, not the goal-gate symptom.**
- Change: run-conclusion construction in the engine — when a goal-gated node fails, carry the node's `stage.failed` message (the invalid_request text + offending attr) into `run.failed.detail` instead of only "goal gate unsatisfied…".
- Effect: triage goes from event-spelunking (the real cause is only in seq 37/39) to a 1-minute fix; the run UI would say "fix `reasoning_effort` on planner to low|high|max" directly.

**5. Add a repair-and-resume path for deterministic config errors.**
- Change: graph/engine — the `planner -> refiner [outcome=failed]` edge is terminal (refiner→exit). Given checkpoints exist (seq 43 captured the failed-planner state), add resume-from-checkpoint UX (or a hold node) so a 10-second human fix continues the run instead of requiring a fresh run with another 33s setup.
- Effect: this exact failure becomes recoverable at node cost instead of full-run cost.

**6. Skip checkpoint commit+push when the diff is empty.**
- Change: engine checkpoint policy — skip commit+push when `files_changed=0`. This run pushed two empty commits (dfb266e, d99a357) at ~4.2s each; ~half the 17.5s active window was snapshot+push overhead for zero changes.
- Effect: ~4–5s saved per no-op node, cleaner run branch, fewer pushes.

**7. Scope skill discovery per node.**
- Change: agent skills config — the planner enumerated 36 skills from 3 dirs including the user-global `/storage/.home/skills` (event seq 31, ~1s of the 1.5s session). Planner needs only `sd`/tracker ops; `implement`, `to-tickets` etc. are irrelevant role leakage.
- Effect: faster session start, smaller system prompt, less risk of the planner invoking an off-role skill.

**8. Degrade invalid tuning attrs instead of failing the node.**
- Change: engine `retry_policy` handling — for `api_deterministic|invalid_request` on an *optional* node attr, retry once with the attr stripped (fall back to run-level default, which here was `reasoning_effort: null` per run settings) rather than failing with no retry.
- Effect: benign attr mistakes like this one survive; `max_attempts=5` currently means nothing for this failure class (`will_retry=false`).

One cost note for calibration: token spend was $0 — the entire waste in this run is time and infrastructure, which is why items 2, 3, and 6 (fail-fast + remove per-run setup + cut checkpoint overhead) dominate the efficiency gains.
