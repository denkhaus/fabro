# Improve review — run 01M2XQJC3TWRMJ1128WRXRQYXH

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (3.4 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-19 21:08+0000 by revisor `fabro_ask`

---

All evidence below is from this run's events (run 01M2XQJC3TWRMJ1128WRXRQYXH, succeeded 21:01–21:04, $0.329 total, 189.8s wall). Seed ids cited are open seeds visible in this run's own `sd ready --assignee fabro --limit 200` output (event seq 46–47), so I checked each against the live tracker listing before recommending.

# Recommendations, ordered by expected impact

**1. Stop feeding the planner the 200-seed firehose — implement seed `fabro-c3b4`.**
- What happened: the planner's first call, `sd ready --assignee fabro --limit 200`, returned **200 seeds / 28,497 bytes** (seq 46). Planner input ballooned to 63,667 tokens and **$0.267 = 81% of the run's total cost, 104.5s = 55% of wall** — to claim a one-line Markdown seed (fabro-048a).
- Change: `.fabro/workflows/develop/prompts/planner.md` step 1 + the sd command table in `.fabro/workflows/develop/prompts/project-facts.md` — take a top-N view (the preflight already renders only 5 candidates; align the prompt with that).
- Seed: **fabro-c3b4** ("Use a top-N sd ready view in the planner instead of the full firehose") — open, covers exactly this.
- Expected effect: removes ~28 KB of dead listing from every planner lap; largest single token/cost cut available in this graph.

**2. Park externally-blocked seeds so they stop ranking first — new seed needed.**
- What happened: top candidate fabro-af22 is blocked on upstream PR #784 — a fact stated verbatim in its own seed body ("UPSTREAM PR OPEN"). The planner still spent **7 tool calls and ~50s (21:02:04→21:02:55) re-proving it**: `sd show`, two `git log` greps (one failed outright on `origin/main`, seq 60), two `rg` searches, a cargo-checkout source grep, and a `web_fetch` of the full 13.4 KB GitHub PR page — then skipped it. Every future run re-derives this until #784 merges. The planner's journal explicitly asks for "a tracker blocker or a needs-upstream label".
- Change: add an `externally_blocked` arm to `.fabro/workflows/develop/scripts/planner-preflight.nu` — when a candidate body names an upstream PR/rev, mark it in the verdict table so the planner skips mechanically (or a `needs-upstream` label the listing filters out).
- New-seed justification: no open seed covers upstream-blocked parking — fabro-8c8a is `needs-user` label semantics only, and this run's journal request hasn't been filed yet.
- Expected effect: eliminates a recurring ~50s/7-call block per planner pass while PR #784 stays open.

**3. Bound the in-flight `fabro_runs_list` call — implement seed `fabro-6b58`.**
- What happened: the planner called `fabro_runs_list {workflow: develop}` with **no `created_since` window and got 93 full run records** (seq 48–49), then scanned them to conclude "only this run is non-terminal."
- Change: `.fabro/workflows/develop/prompts/planner.md` step 4 — pass `created_since` (e.g. 48h) on every in-flight call.
- Seed: **fabro-6b58** ("Bound the planner in-flight check: created_since window plus explicit self-exclusion") — open, names exactly this fix.
- Expected effect: ~90 records × hundreds of tokens off planner context; faster, equally safe exclusion (older runs' PRs are merged or dead).

**4. Batch planner reconnaissance into one shell call — implement seed `fabro-55a7`.**
- What happened: the planner made **11 separate shell calls** (13 messages total); each round-trip re-sent a growing 45–52k-token conversation and added ~5s of inference (per-turn costs: $0.024, $0.057, $0.016, $0.015…).
- Change: `.fabro/workflows/develop/prompts/planner.md` steps 1–4 — chain `sd ready`/`sd show`/git greps into one labeled compound call before the first LLM turn.
- Seed: **fabro-55a7** ("Planner: batch reconnaissance into one shell call (sd ready + sd show + base-branch grep)") — open, covers this.
- Expected effect: fewer LLM round-trips per planner lap — minutes off wall time and roughly proportional cost savings on multi-probe passes like this one.

**5. Add the planner footgun one-liners — implement seed `fabro-6997` (the `rg -rn` trap fired this run).**
- What happened: at seq 65 the planner ran `rg -rn "find_skill_references"` — the exact `-r n` = replace-with-literal-n footgun the implementer prompt already warns about; it silently returned empty (here the target lived in `~/.cargo`, so no corruption — pure luck). At seq 60 it also tried `git log origin/main`, which failed because this clone carries only `origin/denkhaus` — a rule PROJECT_FACTS states but prose alone didn't enforce.
- Change: `.fabro/workflows/develop/prompts/planner.md` — the two one-liners fabro-6997 specifies (never `rg -rn`; fs_hide glob returns empty), plus the sibling "never grep `origin/main`" line.
- Seed: **fabro-6997** — open, covers the rg/fs_hide pair (the origin/main line folds into the same patch).
- Expected effect: removes silent-wrong-result searches and one wasted call+reasoning round per pass.

**6. Silence by-design gate-lint warnings — new seed needed.**
- What happened: the tester output (rendered in the reviewer's preamble) prints **11 warnings every run**: 10 "routing-named property" warnings on `planner-output.schema.json` and the conductor schema — both *intentionally* routing schemas per fabro-9ec3 — plus the recurring "date pin '2026-04-14' older than 45 days" nag. Result: "prompt-lint: ok — 43 files, 11 warnings" trains readers to skip warnings.
- Change: `scripts/qualitygate.nu` prompt-lint — allowlist schemas whose title/description declares routing intent (or accept an inline lint-ignore marker), so only genuine anomalies surface.
- New-seed justification: no open seed covers lint-noise allowlisting — fabro-f18a is a different lint (tool-call JSON examples vs live tool schemas).
- Expected effect: real lint signal stops being buried; gate output becomes actionable at a glance.

**What worked, no change:** the deterministic spine (tracker_guard 0.35s, preflight 4.6s, claim_check 0.09s, evidence 0.32s, closeout 0.41s) and the evidence pipe — the reviewer approved a churn-only loop-work diff with **zero tool calls** in 12.2s/$0.024, exactly what fidelity=summary:high + preamble_inline_max_kb=16 + preamble_output_max_lines=200 were tuned for (fabro-9467/fabro-meta-c9f2). The implementer also ran a clean 3-call pass (dup-run-check, sed edit, grep verify) — the cheapest possible shape for a one-line loop-asset seed.
