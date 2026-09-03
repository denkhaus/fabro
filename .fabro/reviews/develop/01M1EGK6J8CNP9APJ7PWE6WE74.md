# Improve review — run 01M1EGK6J8CNP9APJ7PWE6WE74

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (15.9 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-03 17:43+0000 by revisor `fabro_ask`

---

All findings below are grounded in this run's events, stage transcripts, journals, and the shipped diff (final cost $0.547, 14.2 min active, 2 seeds closed, zero retries).

## Ordered by expected impact

**1. Declare `lesson_capture` in the implementer's `context_allow_keys` — the contract this run shipped is currently self-defeating.**
Seed fabro-f831 (implemented in implementer@2) added a required `lesson_capture` key to the Implemented JSON in `.fabro/workflows/develop/prompts/implementer.md`, but the run's diff never touches `workflow.fabro`, where the implementer node declares `context_allow_keys="implementation_summary,journal"`. Per the graph's own comment (fabro-900e), an undeclared context_updates key "drops visibly with a notice instead of leaking silently" — so the very first run that obeys the new prompt will have its answer discarded as drift. The planner half-suspected this (criterion 5 forced the fact into `implementation_summary` prose "so it survives even where context keys are not forwarded").
*Change:* one line in `workflow.fabro` (implementer node): `context_allow_keys="implementation_summary,lesson_capture,journal"`; optionally add it to the reviewer's `preamble_allow_keys` so review can check it mechanically.
*Effect:* the enforced record-or-skip answer lands durably instead of being dropped on every future implementer pass.

**2. Give the planner a repo-layout map — first-pass orientation burned 40% of the run's cost.**
planner@1: 354.7 s inference / $0.219 of $0.547 total (50,460 input tokens, 4.7 s tool time). The event stream shows ~10 exploratory calls just to locate the target files (`grep evidence`, `ls`+CONTEXT.md, README+justfile, `find / -name evidence.nu`, `git ls-files`, worktree topology) before discovering assets live at `.fabro/workflows/develop/` in the same repo. planner@2, with the layout already in context: 98 s / $0.064.
*Change:* add a 5-line "Repo layout" section to `AGENTS.md` (loaded as memory into every stage) or `prompts/planner.md`: gofib at repo root; dev-loop assets at `.fabro/workflows/develop/`; reviews `.fabro/reviews/`; journals `.fabro/journal/`.
*Effect:* first planner pass drops from ~6 min toward planner@2's ~100 s — roughly −4.5 min wall and −$0.15 per run.

**3. Make evidence.nu show the diff for dev-loop seeds — both reviews this run had to bypass the capture.**
Both seeds target `.fabro/` paths, which evidence.nu classifies as loop churn, so both captures read `seed-work=0 files +0/-0` / "(no seed-work files to diff)". Reviewer@1 verified "by reading evidence.nu and reviewer.md directly"; reviewer@2 had to run its own `git diff` against `diff-base=71366e9` and journaled: "reviewers of workflow-file seeds must read the churn section, not the seed-work section, to find the diff." The per-seed diff-base machinery this run built is exactly what's needed — it's just not applied to churn files.
*Change:* in `.fabro/workflows/develop/scripts/evidence.nu`, when `seed_rows` is empty but `churn_rows` isn't, emit the per-seed (claim-base) diff of changed loop files under a "loop-work diff" section.
*Effect:* reviews of dev-loop seeds verify from context; removes the per-review git detour and the reliance on each planner brief pre-empting the blind spot.

**4. Suppress the `sd prime` banner in workflow stages — it instructs the exact actions the loop forbids.**
planner@1 ran `sd prime` because AGENTS.md memory tells it to (reasoning trace: "per AGENTS.md, run `sd prime`"), pulling a 4.7 KB banner whose "Session Close Protocol" mandates `sd close` + `sd sync && git push` — both reserved for the deterministic Closeout. The planner journaled this as the run's top painpoint: "an LLM skimming the banner could comply."
*Change:* one line in `prompts/planner.md` (and `implementer.md`): "Never run `sd prime` in this loop — its close/push protocol contradicts the role rules; the context brief supersedes it" (or scope the prime instruction in AGENTS.md to interactive sessions).
*Effect:* removes a directly contradicting instruction from every planner context; eliminates the mis-close/push risk and one wasted call per pass.

**5. Make the authorized-exception pattern standing instead of per-brief boilerplate.**
Both briefs this run carried manual override prose — fabro-f831's brief opens "CONTRADICTION RESOLVED: that file's own 'Platform scope is off-limits' section forbids `.fabro/` edits — this brief authorizes exactly that one file" — and implementer@2 journaled the pattern "has now recurred across fabro-7e44 and fabro-f831; worth a platform-level 'authorized exception' convention."
*Change:* in `prompts/implementer.md`'s "Platform scope" section: "Exception: a brief that names a specific platform file authorizes editing exactly that file."
*Effect:* planner briefs shed ~a third of their text (f831's was 2.2 KB, largely authorization), and the implementer no longer has to reconcile a contradiction before starting.

**6. Make null/no-seed path exercise a standing rule for workflow-script seeds.**
implementer@1 found a latent crash: a `string`-typed Nushell param rejects runtime null, which "would have crashed evidence.nu's no-seed-in-progress path" (recorded as mulch `mx-0b966a`). The gate's nu-check is syntax-only ("syntax-clean 7 scripts") and would never have caught it — it surfaced only because this particular brief demanded fallback/null/empty-diff exercises.
*Change:* add to `prompts/implementer.md` step 2: "When editing workflow scripts, exercise the null/no-seed/fallback paths via `nu -c` before finishing."
*Effect:* rarely-hit paths get verified every time, not only when a brief remembers to ask.

**7. Deduplicate the reviewer preamble across cycles.**
reviewer@2's prompt carried the identical ~2 KB evidence capture **twice** plus two tester sections and the prior cycle's closeout block — `preamble_stages_ignore` keeps tester+evidence per visit, and `command.output` is deduped into the evidence stage section rather than replacing it. The graph comment already flags "residual duplication (per-visit stage sections) is engine-side dedup work."
*Change:* workflow-side mitigation until the engine dedups: add `evidence` to the reviewer's `preamble_stages_ignore` (the current capture still arrives via the `command.output` context key, which the prompt already teaches the reviewer to read as a blob ref).
*Effect:* ~2 KB less preamble per review cycle and stale prior-cycle captures disappear; small but compounding on multi-cycle seeds.

**8. Right-size the gate for prompt-only seeds.**
Both tester runs executed the full Go gate (build/vet/test, 11.1 s cold, 1.1 s cached) on a tree where gofib was never touched — both seeds changed only `.fabro/` and `.mulch/` files.
*Change:* path-scope the tester (e.g. a `just qualitygate-lite` running nu-check + sync when only `.fabro/`/`.md` files changed), routed from the same node.
*Effect:* seconds per cycle only — lowest impact here, but it removes a structurally meaningless check from dev-loop seeds.

## What I could not inspect

The live terminal/Slack notification behavior and the auto-merge PR flow (PR #9) were not observable from run events, so I can't ground recommendations about intermediate progress notifications (currently only `run.completed`/`run.failed` fire, per the run settings) — worth checking whether seed-close events would help operators watching 15-minute multi-cycle runs.
