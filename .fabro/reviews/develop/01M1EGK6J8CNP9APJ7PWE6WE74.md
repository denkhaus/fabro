# Improve review — run 01M1EGK6J8CNP9APJ7PWE6WE74

- workflow: develop
- branch integrated: denkhaus-lab
- status: succeeded (completed), 14.2 min, $0.54666
- generated: 2026-09-01 15:13+0200 by `fabro ask` with `scripts/prompts/improve.md`

---

# Improve review — run `01M1EGK6J8CNP9APJ7PWE6WE74`

Run shape (from run events + conclusion): 2 seeds (fabro-7e44, fabro-f831), 2 clean cycles, 0 retries, 15m51s wall / 12m29s inference / 26s tool time, $0.547 total. The loop itself is healthy — first-pass approvals on both seeds, journals on every agent pass. The recommendations below are ordered by expected impact; the top two are correctness bugs in what this run just shipped.

---

**1. The new `lesson_capture` contract is being silently discarded — declare it in the graph.**
Evidence: worker log WARN at 13:08:58, `context_update_dropped ... dropped: lesson_capture`, fired on implementer@2 — the very pass that implemented seed fabro-f831 ("make lesson capture enforceable in the implementer outcome contract"). The seed added a required `lesson_capture` key to the outcome JSON in `prompts/implementer.md`, but the implementer node in `workflow.fabro` still declares `context_allow_keys="implementation_summary,journal"`, so the fabro-900e allow-keys lint drops the structured key; only the prose clause in `implementation_summary` survived. The seed's deliverable is half-dead on arrival.
Change: add `lesson_capture` to the implementer node's `context_allow_keys` in `.fabro/workflows/develop/workflow.fabro` (one line), and add a check to the next improve cycle that greps prompt outcome templates for keys missing from the graph's allow-list.
Effect: the record-or-skip answer round-trips as data the reviewer/planner can verify, instead of living only in prose and a buried WARN.

**2. Evidence captures are blind to seeds that target `.fabro/` itself — make the seed-work classifier seed-aware.**
Evidence: both evidence outputs this run report `integrity: seed-work=0 files +0/-0` and `(no seed-work files to diff)` (evidence@1/@2 stage outputs), because evidence.nu classifies `.fabro/**` as loop churn. Both remaining seeds target exactly those paths, so reviewer@1/@2 had to verify by reading files with tools (their journals say so explicitly: "reviewers of workflow-file seeds must read the churn section, not the seed-work section"). Ironically, fabro-7e44 *built* the per-seed claim-base diff this run and the mechanism never rendered for the only seeds in the tracker.
Change: in `.fabro/workflows/develop/scripts/evidence.nu`, when the in-progress seed exists, classify changed files against the seed's target paths (or simply: loop-classified files that differ vs `diff-base` get a "seed work (loop-classified)" section diffed against the same claim base) instead of path-prefix-only churn classification.
Effect: workflow-file seeds get the same complete-diff review path as product seeds, removes the tool-detour workaround, and satisfies fabro-7e44's own done-criterion ("second capture contains ONLY the second seed's hunks") on the next meta-seed run.

**3. Stop demoting the reviewer's spec to a blob ref.**
Evidence: reviewer@2's preamble shows `current_seed_brief | 2.2 KB; full value: /tmp/fabro/runtime/blobs/f4ec…json; Preview: …` — the 2.2 KB brief (the distilled spec the reviewer judges against) was demoted by the aggregate 24 KB budget while the evidence capture stayed inline via `preamble_inline_max_kb=16`. The reviewer then burned a tool call (shell, 228 ms) reading its own spec.
Change: give contract-critical context keys an inline floor in the demote pass — never demote `current_seed_id`/`current_seed_brief` (or set a per-key `preamble_inline_min` analogous to the existing per-key max). Engine-side, but expressible as a graph lint today.
Effect: one fewer tool round-trip per review and eliminates the risk of a verdict reasoned from a 300-char preview of the spec.

**4. Planner first-visit cost is the run's biggest line item — feed it the facts it keeps rediscovering.**
Evidence: planner@1 = 354.7 s inference, $0.219, 14,936 reasoning tokens, 50,460 input tokens — ~42% of wall time and ~40% of total cost for one claim+brief. planner@2 did the same job in 98 s / $0.064 / 3.9k reasoning. The delta is first-visit repo archaeology (checkpoint-commit subject format, tracker transition semantics, evidence.nu internals) amplified by `reasoning_effort=high`.
Change: persist the durable discoveries into planner-visible memory (the `.mulch/expertise/develop-workflow` mechanism already exists — mx-0b966a landed this run), specifically: claim-commit subject shape `fabro(<run-id>): planner (succeeded)`, status-transition claim detection, and "remaining seeds are loop-classified" context. Keep effort=high; cut the rediscovery, not the reasoning.
Effect: ~3–4 min and ~$0.15 off a typical 2-seed run, with brief quality unchanged (planner@2 is the proof it survives at lower spend).

**5. The gate is blind to file-mode changes — the exec bit on `evidence.nu` shipped broken.**
Evidence: the run's final merged diff carries `old mode 100755 / new mode 100644` on `.fabro/workflows/develop/scripts/evidence.nu`; reviewer@1's journal flagged it ("Unintended hygiene rider … a future commit could restore it") but approved, and nothing downstream restored it — it landed in PR #9.
Change: add a mode-change check to the project's `qualitygate` (fail on tracked scripts losing the executable bit), or make closeout restore/flag mode-only deltas. The deterministic gate is the right owner — the reviewer demonstrably won't block on it.
Effect: no silent permission drift in workflow assets; reviewers stop approving known deviations.

**6. PR-body generation fails every time on the configured model — fix or drop the call.**
Evidence: worker log WARN at 13:10:56: `PR content generation failed; using deterministic fallback ... model=glm-4.7 error=LLM generation failed: No object generated: Failed to parse response as JSON`. The fallback worked (PR #9 exists), but the LLM call is wasted and the failure sits in the post-exit window (exit 13:09:46 → completed 13:11:02).
Change: either repair the JSON-mode prompt/response parsing for the `pull_request.model` (glm-4.7 apparently returns non-JSON), or make the deterministic title/body the default and skip the generation call.
Effect: clean logs, no flaky dependency on the completion path, ~1 min shaved off run finalization.

**7. `sd prime` banner contradicts role rules — role-scope it.**
Evidence: planner@1's journal painpoint: the seeds-cli onboarding banner instructs every session to `sd close` / `sd sync && git push`, directly contradicting the graph's role rules (only deterministic Closeout closes; stages never push). The prompt overrides it, but a skimming model on a bad day complies with the banner.
Change: `sd prime --role implementer|planner|reviewer` (suppress close/push instructions), or a workflow-level note that the Session Close Protocol applies to human interactive sessions only.
Effect: removes a standing instruction-injection vector inside the loop's own tooling.

---

Lower-priority notes (noted, not actioned): the engine serialized two concurrent writes to `implementer.md` during implementer@2 (harmless WARNs, but the agent issued parallel edits to one file); reviewer@2's preamble carried the tester/evidence stage sections from *both* cycles — the known engine-side per-visit dedup work, negligible at 2 KB captures but it will matter at 15–25 KB.

Sources: run events and checkpoints (stage timings, usage, journals, evidence outputs), worker log (4 WARN lines), and the final run diff — all for run `01M1EGK6J8CNP9APJ7PWE6WE74`.
