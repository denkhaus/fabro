# Improve review — run 01M0T2GW0PTNF3CHNQKHER1271

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: failed (10.0 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-04 22:47+0000 by revisor `fabro_ask`

---

All evidence gathered. Here are the recommendations, ordered by expected impact, each grounded in this run's events, logs, and journal.

**Run summary for context** (from run events/logs): 1 seed (fabro-4f3e) claimed, implemented, gate green (1.3s), evidence captured, review Approved, seed closed, tracker empty — every stage succeeded. Total: 8.6 min wall, $0.387 (745K tokens, 644K cache-read; implementer 62% of cost). The run still ended `failed(publish_failed)`.

---

**1. Fix the PR-publish credential — the only real failure in the run.** (Error handling, highest impact)
- What happened: 8/8 git pushes to `fabro/run/01M0T2GW0PTNF3CHNQKHER1271` succeeded with the static token (events seq 89–368), but the GitHub API call to create the PR got **403 Authentication failed** (event `pull_request.failed` seq 369; worker log 14:34:25.9). The run spec requests `integrations.github.permissions.pull_requests: write`, but the static token doesn't actually carry it — push scope ≠ PR API scope.
- Change: run settings (SCM/GitHub credential for this repo): grant the token PR-write (or use an app/installation token for publish). Add a preflight permission check before the workflow starts, so it fails in 2 seconds instead of after 10 minutes of finished work.
- Expected effect: runs like this one end `succeeded` instead of `failed(publish_failed`; auto-merge (squash) can actually run.

**2. Make publish failure non-terminal, with a retry-publish action.** (UX / error handling)
- What happened: at 14:33:47 the graph hit `exit` with everything done — all seeds closed, all commits pushed (final `a01df38`). Then 41 seconds of publish attempts failed and the whole run was marked failed. The work is safe and complete on the run branch; only the PR artifact is missing.
- Change: engine — introduce a terminal state like `succeeded_publish_pending` with a one-click "retry publish" that re-runs only the publish phase against the existing `final_git_commit_sha`.
- Expected effect: a credential hiccup no longer mislabels a successful 10-minute run as failed, and recovery costs seconds, not a full re-run.

**3. Fix PR content generation: model config and JSON-strictness.** (Error handling)
- What happened: worker log 14:34:25.2 — "PR content generation failed … **model=glm-5.3** … Failed to parse response as JSON". The run spec explicitly configures `pull_request.model = "zai:glm-4.7"`, yet glm-5.3 was used: either the setting is ignored or the log mislabels. The strict JSON parse of the generated title/body then failed, forcing the skeleton fallback.
- Change: platform publish path — honor `pull_request.model`, and parse the generation response tolerantly (accept plain text, extract JSON if present).
- Expected effect: PRs get real titles/bodies even when the model wraps output in prose; configured cheap model is actually used.

**4. Order the evidence diff source-first, not alphabetically.** (Graph/script design — the run's only recorded painpoint)
- What happened: the reviewer's journal (`.fabro/journal/reviewer@3.json`) records: "budget cut omitted 2 of 3 seed-work diffs (**main.go, fib_test.go**) while keeping the **README** diff — the least critical file". Root cause is in `.fabro/workflows/develop/scripts/evidence.nu`: line 229 `sort-by path` puts `README.md` before `fib_test.go`/`main.go` (ASCII uppercase first), and `diff-section` line 180 `break`s at the first non-fitting file — so the code diffs, which are largest and most critical, are always the ones cut.
- Change: in `evidence.nu` (lines 164–192 / 229), sort seed-work files code-before-docs (e.g., non-`.md` first) before the budget walk; optionally raise `OUTPUT_BUDGET` (6800) since the capture was only 3.3KB against an 8KB demote threshold with headroom.
- Expected effect: the reviewer judges from context instead of burning tool calls recovering the diff with `git diff 3f7a6ee` — and eliminates the risk of a "Verification blocked" bounce (the failure mode this fast-path was built for, per the graph comments referencing run 01M0SS23MJ's ~3.5 min wasted cycle).

**5. Raise the preamble budget (or trim the verification report).** (Graph design)
- What happened: 4 worker warnings: "preamble values exceed aggregate budget even after demotion; keeping remainder inline" — 14.4KB, 13.9KB, 15.1KB, **21.2KB** vs the 12,288-byte budget set by `preamble_budget_kb=12` in `.fabro/workflows/develop/workflow.fabro`. The drivers: `implementation_summary` 4.1KB (the per-criterion PASS report), brief 2.1KB, evidence 3.3KB. The blob-ref mechanism worked (reviewer read them), but every blob round-trip is a tool call and a verification-blocked risk.
- Change: `preamble_budget_kb=12` → 18 in the `graph [...]` attrs of `workflow.fabro`. At this run's scale ($0.39 total, context window at 1.7% used) the larger preamble is free.
- Expected effect: evidence and summary arrive inline; reviewer's tool time (1.8s, already spent mostly on recovery) drops toward zero; warnings disappear.

**6. Fix the progress counter: "6 of 5 stages".** (UX)
- What happened: the snapshot reads "6 of 5 non-meta stages completed" because planner legitimately visited twice (claim + close/tracker-empty) while the denominator counts unique nodes (5).
- Change: engine progress display — count stage *executions* in both numerator and denominator (or show "5/5 nodes, 6 visits").
- Expected effect: no more >100% progress on every multi-seed run, which is every run of this graph by design.

**7. Keep planner `reasoning_effort=high` only where it pays.** (Cost/latency, lowest impact)
- What happened: planner@1 burned 102.5s inference / 5,042 reasoning tokens to claim one seed — but that deep reasoning caught the seed's real spec contradiction ("Default 0" vs pinned 1-based indexing) and pre-resolved it, which is why the whole cycle passed in one review round. Planner@2 (close seed + confirm tracker empty) used 22.5s/$0.018 for a trivial mechanical task at the same `high` effort.
- Change: since per-visit effort isn't configurable, add to `prompts/planner.md`'s verdict-handling section: "when the only action is closing an approved seed or confirming an empty tracker, skip analysis and emit the JSON immediately."
- Expected effect: shaves ~15–20s and a little cost off every loop iteration's close pass; claim passes keep full reasoning.

---

**What worked and should not be changed** (evidence this run): the agent-node reviewer with tools (recovered the cut evidence itself and still Approved); the inline PASS/FAIL verification report (reviewer approved against it item-by-item); the stage-journal hook (the reviewer painpoint landed durably in `reviewer@3.json` — the bridge noted as missing in commit b74c4ea now works); the pre-baked runner image (setup was 849ms vs the ~24.5s this replaced); deterministic gate/evidence nodes (1.3s + 0.4s).

**Not inspectable from here:** the platform-side publish code and the actual token scopes live outside this run's workspace — recommendations 1–3 name the config surface but the fix itself is in the fabro engine/credentials, which I could only observe via logs and events.
