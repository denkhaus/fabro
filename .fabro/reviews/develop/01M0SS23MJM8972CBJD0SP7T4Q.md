# Improve review — run 01M0SS23MJM8972CBJD0SP7T4Q

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: failed (7.9 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-02 19:18+0000 by revisor `fabro_ask`

---

All findings below are from this run's events, worker log, and stage transcripts (run 01M0SS23MJM8972CBJD0SP7T4Q: 9m36s wall, $0.257, 337k tokens, 5 stages × ~2.5 cycles, final status `failed(publish_failed)`).

## 1. Fix the reviewer's evidence pipe — it rejected (and later approved) work it never saw
**What happened:** reviewer@1 rejected green, gate-passed work as "Changes requested" because the evidence capture and seed spec arrived as blob refs it couldn't read — its own feedback says *"format-locked to JSON-only output with no opportunity to read them."* The node is a prompt node (`shape=tab`, `tool_time_ms: 0`), yet `prompts/reviewer.md` tells it *"You DO have tools… That file is IN YOUR SANDBOX — read it."* The handler can't make tool calls; the policy mandates reading; contradiction. Worker log shows the cause: `preamble values exceed aggregate budget… total_bytes=12453→21931 budget=12288` at every agent stage from 11:43:40 onward. Worse, reviewer@2 **approved** while the capture was still a blob ref — the diff-vs-spec review the loop was built for never actually happened in either pass; approval rested on the implementer's self-report.
**Change:** in `.fabro/workflows/develop/workflow.fabro`: (a) raise `preamble_budget_kb` from 12 to ~24 (actual demand was 12.4–21.9KB), and/or (b) run the reviewer as an agent node with read-only tools so blob refs are readable.
**Expected effect:** first-cycle approvals with real diff inspection. The workaround second cycle (planner@2→reviewer@2) cost $0.112 of $0.257 — 43% of run spend and ~3.3 of 7.5 loop minutes — for zero code change.

## 2. Don't let a publish 403 mark achieved work "failed"
**What happened:** the loop fully succeeded — seed `fabro-e6df` closed, tracker empty, 13 commits pushed (git.push `success: true`) — then the run went `failed(publish_failed)`: PR-body LLM generation failed to parse (`Failed to parse response as JSON`), the deterministic fallback kicked in, and PR creation died on `Authentication failed creating pull request (403)`, category deterministic. The only artifact of a completed goal is a "failed" run and an unmerged branch.
**Change:** add a pre-run credential check for PR scope (one cheap API call in prepare), and in the publish step treat "branch pushed, PR failed" as completed-with-publish-warning (retryable) instead of run failure; separately fix the PR-body JSON contract for glm-5.3 since the fallback masked it.
**Expected effect:** run status reflects loop outcome; a re-run of publish only, not ambiguity over a "failed" run whose tracker is already empty.

## 3. Stop re-rendering every past cycle in preambles
**What happened:** reviewer@2's prompt contains the tester and evidence stage sections **twice** (once per cycle), plus superseded context keys (`review_feedback` 1.6KB, `journal`, old brief) that persist after consumption. That linear growth is exactly what blew the 12KB budget (log: 12.4→18.1→19.2→21.7→21.9KB).
**Change:** in `workflow.fabro`, render only the **latest** visit of command stages for reviewer/implementer preambles, and clear consumed keys (`review_feedback`, `review_verdict`, stale `implementation_summary`) after the planner handles them.
**Expected effect:** preambles stay under budget at any cycle count (`max_visits=20` today would make cycle 5+ unreadable).

## 4. Add an "evidence unreadable" route instead of mislabeling it "changes_requested"
**What happened:** reviewer@1 explicitly said *"The Implementer should not re-do the README; simply resubmit"* — yet the only available label routed the whole thing back through planner→implementer, burning one of the implementer's 10 visits on a no-op verification pass the planner had to invent.
**Change:** in `workflow.fabro` + `prompts/reviewer.md`: third verdict `Evidence unreadable` with a direct edge reviewer→evidence (re-capture/inline) or engine retry, bypassing the implementer.
**Expected effect:** verification failures recover in seconds without consuming implementer visits or ~$0.09 of planner+implementer tokens.

## 5. Cut the 88.5s `mise install` that the runner image was built to eliminate
**What happened:** setup ran `mise install` for **88,546ms** (of 90.9s total setup, ~20% of the 9m36s run) even though the runner image header promises it "becomes a no-op version check instead of ~24.5s setup". The image was present (snapshot ready in 45ms), so mise re-resolved/downloaded — almost certainly a version drift between the dockerfile's pinned set and `.mise.toml`, which the dockerfile comment warns "MUST match".
**Change:** align/verify versions in the runner dockerfile (`settings.environment.image.dockerfile`) against `.mise.toml`, or drop `mise install` from `prepare.steps` and assert versions instead.
**Expected effect:** ~90s saved per run, deterministic.

## 6. Prompting: stop routing evidence through context values
**What happened:** planner@2's workaround brief ordered the implementer to inline a full verification report into `implementation_summary`; that key grew to 2.6KB and was itself blob-offloaded at planner@3/reviewer@2 — pushing bulk through context updates made the budget problem worse.
**Change:** in `prompts/planner.md` and `prompts/implementer.md`: cap summaries ("≤ ~500 chars; evidence belongs in the capture or repo, never in context_updates") and delete the "inline report" instruction once #1 lands.
**Expected effect:** stable, small context keys; no self-inflicted blob refs.

## 7. Tool ergonomics: bake `rg` into the image; the planner wasted calls rediscovering files
**What happened:** planner@1 tried `rg` → `command not found`; the native `grep` tool then returned empty on `main.go` twice; it fell back to `find`/`head`. ~5 of its tool calls were re-orientation a familiar tool would have done in one.
**Change:** add ripgrep to the runner dockerfile toolset (one `apt-get`/`mise` line).
**Expected effect:** fewer wasted exploration calls per planner pass.

## 8. Minor hygiene (cheap, do in passing)
- **Skills noise:** every one of the 7 agent sessions loaded ~35 irrelevant Matt-Pocock skills (~1.5k tokens each, never activated). Scope skill discovery off for the develop workflow in run settings (`agent` block) — less prompt noise across the loop.
- **Empty journal heartbeats:** the `stage-journal` hook wrote 13 identical `{"data": {}}` files into every checkpoint diff (the "loop-churn=10 files +82/-1" the evidence script then reports). Either populate `data` (e.g., with the stage's painpoints) or write only non-empty journals in `scripts/stage-journal.nu`.
- **Checkpoint cost:** metadata snapshots ran ~1.8–2.3s at all 13 checkpoints (~25s total); skipping them on non-agent stages (tester/evidence) would trim wall time for free.

**Not broken (no change):** the gate design — `just qualitygate` ran green in 2.1s/0.9s with zero retries; the seed spec contradiction-annotation in planner@1's brief worked exactly as intended; artifact hygiene rules were followed (builds in `/tmp`, worktree clean).
