# Improve review — run 01M0T2GW0PTNF3CHNQKHER1271

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: failed (10.0 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-02 19:23+0000 by revisor `fabro_ask`

---

All graph stages in this run actually succeeded — the loop claimed seed `fabro-4f3e`, implemented it, passed the gate, got an Approved review, closed the seed, and exited "Tracker empty" with 12 files (+265/−34) committed and pushed. The `failed(publish_failed)` status, the reviewer's one recorded painpoint, four preamble-budget warnings, and the per-stage timings give plenty to work with. Recommendations, ordered by expected impact:

---

**1. Stop terminal-failing the run on PR publish errors — this run's only real failure.**
Evidence: worker logs show `git.push` succeeded at 14:26:31 (static token), then at 14:34:25 `Pull request creation failed error=Authentication failed creating pull request (403)`, and at 14:34:28 `Workflow run failed reason=publish_failed`. A fully successful, $0.39, 8.7-minute run is archived as `failed`.
Change: in the run/workflow publish settings (the `pull_request` block referenced by `.fabro/workflows/develop/workflow.toml`'s graph config), either (a) disable `pull_request.enabled` for this environment, or (b) engine-side, downgrade publish failure to `succeeded_with_errors` plus a compare URL (`.../compare/base...fabro/run/<id>`) instead of `failed`.
Effect: run status stops lying; no more "everything worked but the report says failed" triage. (I could not inspect the token/permission internals behind the 403 — logs are redacted — so the credential-scope fix itself is outside what I can verify.)

**2. `.fabro/workflows/develop/scripts/evidence.nu` — prioritize source files in the diff walk, and give the diff more room.**
Evidence: this is the run's one journal painpoint (`.fabro/journal/reviewer@3.json`): "budget cut omitted 2 of 3 seed-work diffs (main.go, fib_test.go) while keeping the README diff — the least critical file… the review hinged on exactly the files that were cut." The reviewer had to recover them itself via `git diff 3f7a6ee`. Root cause in the script: line 229 sorts seed rows alphabetically (`README.md` first), and `diff-section` (lines 164–192) includes files whole-or-nothing in that order, so README (~1.3 KB) consumed the ~2 KB allowance and both Go files got the "budget cut: 2 of 3 files omitted" disclosure. Meanwhile the whole capture was only 3.3 KB against a self-imposed 6.8 KB budget (line 34) and an 8 KB engine demote threshold.
Change: sort seed files code-first (`.go`/source before docs) before the diff walk, and raise `OUTPUT_BUDGET` from 6800 to ~7600 by trimming `SPEC_CAP` (2200) — the spec is re-fetchable via `sd show`.
Effect: the reviewer verifies from the capture directly; no tool-recovery detour (reviewer tool_time 1.7 s would otherwise grow), and the "Verification blocked" path stops being one bad ordering away from firing.

**3. Make the preamble budget coherent — 12 KB configured vs. 14–21 KB actual.**
Evidence: four worker-log warnings: `preamble values exceed aggregate budget even after demotion; keeping remainder inline` — totals 14421, 13877, 15088, and 21191 bytes against `preamble_budget_kb=12` (planner@2's prompt carried ~21 KB).
Change: in `workflow.fabro` graph attrs, raise `preamble_budget_kb` from 12 to ~24 (the loop's real steady-state context is `current_seed_brief` 2.1 KB + `implementation_summary` 4.1 KB + evidence 3.3 KB + gate output), or stop re-injecting the full `implementation_summary` into the planner's preamble (planner only needs the verdict).
Effect: the demotion policy becomes predictable — no more over-budget inline values, fewer blob-ref round-trips for the reviewer, and smaller-than-necessary planner prompts stop being a silent config lie.

**4. Scope skill discovery — ~1.55k tokens of irrelevant skills in every LLM turn.**
Evidence: from run events, every stage's context-window breakdown carries ~1,540–1,680 tokens of `skills` (the ~40-entry matt-pocock set from `/storage/.home/skills`), and `activated: []` in every stage — nothing was ever used. The planner alone had 6 LLM turns; across ~10+ turns that's >15k wasted tokens per run.
Change: restrict skill source dirs for this workflow to `.fabro/skills` (agent settings in the run spec / `workflow.toml`), or disable the global set.
Effect: measurable token/cost reduction per run and less prompt noise competing with role instructions.

**5. Lint seeds for spec contradictions before they enter the tracker.**
Evidence: the seed text said `-start` "Default 0" while gofib's 1-based indexing is pinned by README/CONTEXT/tests. The planner resolved it well, but at real cost: planner@1 burned 102.5 s inference and $0.067 — the two long reasoning blocks in the event stream (seq 62, 70) are almost entirely the (A)-vs-(B) indexing deliberation. That's ~40% of planner time on a defect in the seed, not the plan.
Change: add a contradiction check at seed creation (wherever `fabro-4f3e`-style seeds are authored) comparing the spec's examples against pinned behavior in README/CONTEXT.md.
Effect: ~1.5 min and ~$0.05 saved per ambiguous seed, and one less source of reviewer ping-pong downstream.

**6. Forbid `sd prime` in the planner/implementer prompts.**
Evidence: the planner ran `sd prime` unprompted (event seq 38); its output (seq 42) injects a "Session Close Protocol" commanding `sd close …`, `bun test && bun run lint`, `git push` — instructions that directly contradict the role contracts (implementer: never close seeds, never push, commands go through `just`). Harmless this time, but it's adversarial-looking context injected mid-loop.
Change: one line each in `.fabro/workflows/develop/prompts/planner.md` and `implementer.md`: "Do not run `sd prime` — the workflow pre-primes context and its close-protocol text conflicts with this role."
Effect: removes conflicting third-party instructions from agent context and saves a call.

**7. Fix the progress counter (engine projection).**
Evidence: the implementer's prompt read "Pipeline progress: 0 of 5 stages completed" after the planner had completed; the final projection says "6 of 5 non-meta stages completed" because planner ran twice. The counter mixes stage completions (with revisits) against a fixed node denominator.
Change: compute progress as unique completed non-meta nodes / total, not cumulative stage completions.
Effect: honest progress UX; today the numbers are impossible to interpret mid-loop.

**8. Minor, same path as #1: PR content generation failed before the 403.**
Evidence: worker log at 14:34:25 — `PR content generation failed… model=glm-5.3 error=Failed to parse response as JSON`, even though the run spec configures `pull_request.model = "zai:glm-4.7"`. The deterministic fallback worked, so impact was small, but the configured model apparently wasn't honored.
Change: honor `pull_request.model` and retry once without strict-JSON output before falling back.
Effect: meaningful PR titles/bodies whenever publishing is fixed; removes a silent config override.

---

**What worked and needs no change** (worth knowing so you don't "fix" it): the graph economics are excellent — deterministic tester 1.3 s, evidence capture 0.4 s, and the final planner pass that closed the seed and detected tracker-empty cost just 22.5 s / $0.018 thanks to `preamble_stages_ignore`; the implementer's per-criterion PASS/FAIL report let the reviewer approve mostly from context (85 s total); and the journal bridge demonstrably works — the reviewer's painpoint landed 1:1 in `reviewer@3.json` while empty `data` for other stages correctly reflects that they declared nothing.
