# Improve review — run 01M0TKB7H7DCWCR8TZY6SZPXD2

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (5.8 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-04 23:05+0000 by revisor `fabro_ask`

---

All evidence below is from this run's events (`fabro_run_get` / `fabro_run_events`), the worker log (`fabro_run_logs`), and workspace files. Context for sizing: 299 s wall, 236.5 s inference, 18.7 s tool time, $0.226 total, 0 retries, one seed (`fabro-cfb6`) taken claim→close in a single clean cycle.

## Recommendations, ordered by expected impact

**1. Raise `preamble_budget_kb` from 12 to ~32 — graph attr in `.fabro/workflows/develop/workflow.fabro`.**
What happened: the engine warned at planner@2: *"preamble values exceed aggregate budget even after demotion; keeping remainder inline total_bytes=17262 budget=12288"* (worker log 19:23:31). At reviewer@1, three values were blob-demoted (evidence capture 14.3 KB, `current_seed_brief` 1.5 KB, `implementation_summary` 2.0 KB), so the reviewer spent a tool call + LLM turn just re-ingesting its primary input. glm-5.3 ran at ~2 % of a 1 M context with 82 % cache-read tokens — inline delivery is nearly free, while blob round-trips cost a turn each and recreate exactly the "verification could not be completed" failure class the `Verification blocked` edge was built to avoid. Expected effect: reviewer judges straight from context (one fewer turn, ~10–20 s and ~$0.01–0.02 per review), and the overflow-warning class disappears. Don't shrink the capture instead — completeness is `evidence.nu`'s deliberate design.

**2. Fix the PR finalize path — run spec `pull_request` block.**
What happened: PR content generation burned 11.6 s (19:24:04→19:24:16) and then **failed** — *"Failed to parse response as JSON"* — falling back to a skeleton title/body for PR #1; immediately after, auto-merge failed (*"Auto merge is not allowed for this repository"*) despite `auto_merge=true` in the spec. Also note the spec pins `model=zai:glm-4.7` for PR content but the failure is tagged `model=glm-5.3` — the override appears not to be honored (I can't verify which from here; the failure is real either way). Concrete change: give the PR-content call the same `output_retries=2` treatment agent nodes get (or a plain-text schema), and set `auto_merge=false` until the repo setting is on. Expected effect: ~12 s off every run's tail, real PR descriptions, no silently broken auto-merge promise.

**3. Cap reviewer reasoning and stop it re-running the gate — `reviewer` node attrs + `.fabro/workflows/develop/prompts/reviewer.md`.**
What happened: reviewer@1 cost 61 s inference and 3,424 reasoning tokens — 26 % of run inference time and $0.0587 of $0.226 — to approve a green, fully-evidenced 3-file diff, rebuilding the binary and re-running `go test`/`go vet` ("I independently confirmed…") despite its prompt saying "Judge primarily from the context; fall back to tools when the context is incomplete." The same repo already proved the effort lever works: implementer at `reasoning_effort="low"` cost $0.109 here vs $0.213 in the prior run (per the graph's own comment). Concrete change: add `reasoning_effort="low"` (or medium) to the reviewer node, and one line in `prompts/reviewer.md`: "The tester stage already ran the gate green; do not rebuild or re-run it unless the evidence contradicts the tester output." Expected effect: reviewer roughly halved (~30 s, ~$0.03 saved per review), duplicated verification eliminated on every seed.

**4. Stop the implementer running the full gate — brief template in `.fabro/workflows/develop/prompts/planner.md` + step 4 in `prompts/implementer.md`.**
What happened: the implementer's summary says "Smoke checks and `just qualitygate` both green" and its PASS report includes "just qualitygate green (ran as smoke check: passed)" — it ran the full gate even though `prompts/implementer.md` says not to, because the planner's brief template ends every brief with a bullet "Gate: `just qualitygate` green", which reads as an implementer acceptance criterion. It cost only ~1 s here (cached), but gates scale with repo size while this graph loops per seed. Concrete change: delete the "Gate green" bullet from the brief format in the planner prompt (it's the tester node's invariant, not the implementer's criterion), and make implementer step 4 absolute: "never run `just qualitygate`; a smoke check is `go build` or one `go test -run`." Expected effect: no gate double-run per seed; grows in value with gate duration.

**5. Skip per-stage checkpoint snapshot/push for non-mutating stages — checkpoint policy (run spec / painpoint channel).**
What happened: 8 metadata snapshots (1.78–2.4 s each) plus a git push per checkpoint ≈ 16 s serialized between stages — 5.3 % of wall on a 6-minute run — and the `tester` (1.03 s) and `evidence` (0.41 s) stages mutate nothing but journal lines and blobs, yet still pay full snapshot+commit+push. Concrete change: checkpoint only stages whose diff touches the worktree (planner, implementer) plus finalize, or exclude `.fabro/journal` + `.fabro/blobs` from the per-stage snapshot. Expected effect: ~6–10 s per run now, proportional savings on multi-seed runs where the loop revisits nodes.

**6. One-source confirmation for the tracker-empty route — `.fabro/workflows/develop/prompts/planner.md`.**
What happened: planner@2 closed the seed, then ran both `sd ready` **and** `sd list --format json` ("both confirm the tracker is now empty") — 5 LLM turns for close+confirm. The sd table already says "If it answers the question, do NOT also run `sd list`", but the terminal decision invites belt-and-braces. Concrete change: add to the Tracker-empty contract: "For the tracker-empty decision, `sd ready` returning nothing is sufficient — do not also run `sd list`." Expected effect: one tool call + one turn saved per end-of-backlog run; consistent with the rest of the prompt.

**7. Two small UX fixes (platform-side, emit via the painpoint channel).**
(a) The run snapshot reports "Progress: **6 of 5** non-meta stages completed" — visits (7 completions, planner×2) vs distinct nodes (5) are mixed; a visits-aware display ("planner×2, 7 visits / 5 nodes") removes the impossible fraction. (b) `seed_cycles` carries noise keys (`start`, `evidence`) the planner prompt never uses — filter to `{planner, implementer, tester, reviewer}` so the guard object is self-explanatory. Expected effect: trivial cost, removes two recurring "wait, what?" moments for anyone reading run state.

**What I could not inspect:** whether the per-stage checkpoint behavior and `seed_cycles` key set are engine-fixed or spec-configurable (recommendation 5/7 may belong in the platform improve workflow rather than this graph), and whether the PR-model override (`zai:glm-4.7` vs logged `glm-5.3`) is a config or logging bug — both are visible only as symptoms in this run's log.
