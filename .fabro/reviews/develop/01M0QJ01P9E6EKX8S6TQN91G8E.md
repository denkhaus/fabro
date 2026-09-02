# Improve review — run 01M0QJ01P9E6EKX8S6TQN91G8E

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: failed (0.3 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-02 17:51+0000 by revisor `fabro_ask`

---

# Recommendations — run `01M0QJ01P9E6EKX8S6TQN91G8E`

**What actually happened (from run events + worker logs):** The run spent 33.4s on sandbox + setup (`mise install` alone: 26.6s), reached the `planner` agent at 14:57:38.5, and died **1.5s later** with `invalid_request: model 'glm-5.3' does not support reasoning_effort 'medium'; allowed values: low, high, max` — 0 tokens, 0 inference time. It then spent **~14 more seconds** on two metadata snapshots (~1.75s each), two empty commits (`dfb266e`, `d99a357`, both 0 files changed) and two remote pushes, before reporting failure as the secondary error `"goal gate unsatisfied for node planner and no retry target"`. The refiner ran 215ms emitting a 44-byte "no staged painpoints" message (matching `refiner.nu:100`). Skills discovery enumerated **36 skills from 3 dirs including `/storage/.home/skills`** (personal skills like `ask-matt`, `scaffold-exercises`) for a planner that only needs `sd`.

---

### 1. Fix the root cause: `reasoning_effort="medium"` is invalid for the configured model
- **Change:** In `.fabro/workflows/develop/workflow.fabro` line 29 (planner node), change `reasoning_effort="medium"` to `"low"` (glm-5.3 allows low/high/max) — or delete the node-level pin and let the run-level `model.controls` govern, which is currently `null`.
- **Effect:** The planner's first LLM call succeeds; this exact failure (run events seq 37/39, `will_retry=false`) becomes impossible. This is the single change that converts this workflow from "0 of 4 seeds touched" to functional.

### 2. Validate node-level model controls at submit time, not 35 seconds into the sandbox
- **Evidence:** The error is purely static: graph pins `medium`, provider allows `low|high|max`, run settings show `controls.reasoning_effort: null`. The engine discovered it only after Docker spin-up (3.1s), `mise install` (26.6s), and an init meta snapshot (2.0s).
- **Change:** Add a capability check (engine submit path, or extend the existing `.fabro/workflows/develop/scripts/sync-check.nu` to cross-check node attrs against the resolved provider/model) that rejects the run before sandbox creation.
- **Effect:** Config-class errors fail in <1s with the provider's exact message, instead of a 50.7s run that costs a cloned sandbox and a pushed branch.

### 3. Degrade instead of dead-ending on `invalid_request` for a single unsupported parameter
- **Evidence:** `planner` had `max_attempts=5` but `will_retry=false` (category `deterministic`), and `run.model.fallbacks = {}` — one parameter mismatch consumed the whole run even though the error message itself contained the remedy ("allowed values: low, high, max").
- **Change:** Configure `fallbacks` in the run's model settings, and/or add engine behavior: on `api_deterministic/invalid_request` naming an unsupported parameter, retry once with that control dropped.
- **Effect:** A model swap or control drift degrades one call (default effort) instead of failing a goal-gated run.

### 4. Surface the root cause in the run's terminal status
- **Evidence:** The run's visible failure is `workflow_error: goal gate unsatisfied for node planner and no retry target` (run.failed, seq 58); the actual cause lives only in the stage detail. The goal-gate error masks the invalid-request error.
- **Change:** In the engine's conclusion mapping (or by dropping `goal_gate=true` on `planner` in `workflow.fabro:27`, since planner failure already has an explicit `Planner failed → refiner` edge), make the terminal reason the **original** failure signature `planner|…|invalid_request`.
- **Effect:** A user reading the run list sees "model rejected reasoning_effort" — actionable in seconds — instead of a goal-gate message that points nowhere.

### 5. Stop loading personal skills into role-scoped agent stages
- **Evidence:** `agent.skills.discovered` (seq 31) pulled 36 skills from `/storage/.home/skills` + repo dirs into the planner session (~1.0s of discovery, plus prompt bloat); the planner/implementer prompts even carry defensive rules about skill-token crashes ("never write a bare slash-word"), which is overhead defending against this noise.
- **Change:** Scope skill source dirs per workflow/node in agent settings to `.fabro/skills` (and drop `/storage/.home/skills` for the develop workflow).
- **Effect:** Smaller preambles (budget is already tight at `preamble_budget_kb=12`), less distraction for role prompts, and removal of the crash class the prompts currently work around.

### 6. Don't checkpoint-commit and push after a zero-diff, zero-token doomed stage
- **Evidence:** From planner failure (14:57:40.0) to run.failed (14:57:53.9): two meta snapshots, two commits with `files_changed: 0`, two pushes to `fabro/run/…` — ~14s, over half the workflow's 12.7s wall time, producing only branch pollution.
- **Change:** In the engine's checkpoint path, skip the commit/push when the stage diff is empty and the run has no retry target (keep the event log as the record).
- **Effect:** Failed runs terminate in ~2s after their terminal stage; no empty `dfb266e`/`d99a357`-style commits to clean up.

### 7. Bake tool versions into the runner image
- **Evidence:** `mise install` took 26.6s of the 28.5s setup (run events seq 15) on the purpose-built `fabro-runner:mise` image — for a run whose entire useful life was 1.5s.
- **Change:** Pre-install the `.mise.toml` toolset in the `fabro-runner:mise` Dockerfile (or mount a mise cache volume in the environment config).
- **Effect:** ~27s saved per run; matters most precisely for short/failed runs where setup dominated 2:1.

---

**Not addressed (insufficient evidence):** The tester/evidence/reviewer nodes never executed in this run, so I can't ground recommendations about gate behavior, evidence capture ordering, or review-cycle prompting in what happened here — the graph comments referencing runs `01M0NJZGX…` and prior cycles are design notes, not events of this run.
