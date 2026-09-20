# Improve review — run 01M0SS23MJM8972CBJD0SP7T4Q

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: failed (7.9 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-04 22:41+0000 by revisor `fabro_ask`

---

## What actually happened in this run (grounding)

One seed (`fabro-e6df`, gofib README) went through the loop **twice**. The first pass was clean — implementer wrote `README.md` (+111/−0), `just qualitygate` green at 11:43:29, worktree clean — but the reviewer rejected it as *"verification-uncertainty"*, explicitly stating the evidence/spec blob refs were *"never read into this review pass"* because the pass was *"format-locked to JSON-only output"* (from run events, `reviewer@1` transcript + its journal painpoint). The whole second loop (planner@2 → implementer@2 verification-only → tester@2 → evidence@2 → reviewer@2 approve) existed only to work around that. Then, after all 12 stages succeeded and the seed was closed, the run ended `failed(publish_failed)` on a PR-creation 403 (from run logs). Total: 7m03s wall, $0.257, of which the wasted loop is ~152s and ~$0.107 (~42% of model spend).

## Recommendations, ordered by expected impact

**1. Fix the reviewer's evidence pipe — it was promised tools it doesn't have.**
- Evidence: `reviewer@1`/`reviewer@2` ran with `handler=prompt`, `tool_time_ms=0` — the prompt in `.fabro/workflows/develop/prompts/reviewer.md` says *"You DO have tools… That file is IN YOUR SANDBOX — read it… an unread blob ref is [grounds for rejection]"*, but prompt mode made tool reads impossible, so the 4.6 KB capture at `blob://…1e3f5404` was never readable. Worker logs show why it was blobbed: `preamble values exceed aggregate budget… total_bytes=12453 budget=12288` — the graph's `preamble_budget_kb=12` in `workflow.fabro` is smaller than one gate log + one evidence capture.
- Change: in `workflow.fabro`, either (a) raise `preamble_budget_kb` to ~24 so the seed-work diff and seed spec stay inline (they're 4.6 KB together — they fit), or (b) switch the `reviewer` node to an agent handler so the "read the blob" instruction is executable. Option (a) is one line.
- Expected effect: eliminates the entire false-rejection loop — here that was ~2.5 min, ~$0.11, 5 extra stage visits per affected cycle. Also removes a review-integrity hole: `reviewer@2` ultimately approved based on the *implementer-authored* inline verification report (implementer@2's 2.6 KB `implementation_summary`), i.e., trust-by-assertion instead of reading the diff.

**2. Don't let a publish-time 403 retroactively fail a fully green run.**
- Evidence: run logs — `Pull request creation failed error=Authentication failed creating pull request (403)` at 11:48:33, after every stage succeeded, the seed was closed, and `git.push` succeeded (`success: true` at seq 92). Status became `failed(publish_failed)`; no PR exists despite `integrations.github.permissions: pull_requests: write` in the run spec.
- Change: preflight the PR credential before stage 1 (or at least before the final exit edge), and on publish failure mark the run `succeeded_with_warnings` + a pending control instead of `failed`.
- Expected effect: 7 minutes of verified green work stops being reported as a failure; the user gets an actionable auth error at t≈0 instead of after the loop, and the run branch (already pushed) remains the deliverable.

**3. Make `mise install` the no-op the runner image claims it is.**
- Evidence: run events — setup `mise install` took **88,546 ms** (of 90.9s total setup), while the image dockerfile's own header says baked-in tools should make it *"a no-op version check instead of ~24.5s setup."* That's ~20% of end-to-end wall time, more than all agent tool time combined (18.7s).
- Change: align the versions in the inline `image.dockerfile` (`bun@1.4.0`, `nushell@0.115.0`, `go@1.27.0`, `just@1.58.0` installed globally into `MISE_DATA_DIR=/mise`) with the cloned repo's `.mise.toml`, or bake the workspace `.mise.toml` resolution into the image; verify with one timed run.
- Expected effect: ~88s saved per run (~20% wall), deterministic setup.

**4. Deduplicate repeated stage sections in preambles.**
- Evidence: `reviewer@2`'s prompt contains the **tester and evidence sections twice each** (cycle 1 and cycle 2 copies); `planner@3`'s prompt lists tester+reviewer twice. Worker logs show preamble totals growing 12.4→21.9 KB across visits against the 12 KB budget — the growth is what pushed the evidence capture into blob-offload in the first place.
- Change: in `workflow.fabro`'s preamble rendering (or a `latest_visit_only` per-stage attr), keep only the most recent section per stage for tester/evidence.
- Expected effect: preamble stays under budget as loops accumulate — this is what would have kept recommendation 1 fixed even on long multi-seed runs; also cuts per-cycle token cost and model confusion.

**5. Clear `review_feedback` when the verdict flips to approved.**
- Evidence: at `planner@3` (from run events), context still carried the full stale cycle-1 rejection text (`review_feedback`, 1.6 KB) alongside `review_verdict=approved`; the reviewer's Approved template in `prompts/reviewer.md` only sets `review_verdict`, never resets the feedback.
- Change: add `"review_feedback": ""` to the Approved JSON template in `.fabro/workflows/develop/prompts/reviewer.md`.
- Expected effect: stale "changes requested" instructions can't leak into the next seed's re-plan; smaller, unambiguous planner context.

**6. Skip the gate/evidence pair on no-change verification resubmits.**
- Evidence: `tester@2` (922 ms) and `evidence@2` (384 ms) re-ran on a byte-identical worktree — implementer@2 changed nothing by design. Cheap here, but the graph routes `implementer -> tester` unconditionally in `workflow.fabro`.
- Change: add an `Implemented (no changes)` label path from implementer straight to reviewer (or condition the tester edge on a dirty worktree).
- Expected effect: saves one full gate run per procedural resubmit — scales with gate weight (this project's gate is 1–2s; a real build/test gate would be minutes).

**7. Remove sandbox tool drift that wasted planner turns.**
- Evidence: `planner@1` events — `rg: command not found` (seq 49), then two `grep` calls with `glob_filter: "main.go"` returning empty (seq 58–59), then a recovery `find/head` call; ~3 extra LLM turns rediscovering what tools work.
- Change: add `ripgrep` to the runner image dockerfile, and note in `.fabro/workflows/develop/prompts/planner.md` that the grep tool's `glob_filter` needs a glob (e.g. `*.go`), not a bare filename.
- Expected effect: fewer wasted inference turns per planning visit (~10–15s and a few thousand tokens each).

One thing I could not verify from inside this run: whether the 403 stemmed from token scope, expiry, or repo settings — the redacted logs stop at "Authentication failed"; that diagnosis has to happen in the platform/credential layer, which this run's sandbox cannot inspect.
