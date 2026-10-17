# Improve review — run 01M55KRG2C3DAT21A0VSN240BD

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (3.2 min, revisor pass — reason and cost in run detail)
- generated: 2026-10-17 19:15+0000 by revisor `fabro_ask`

---

Tracker note up front: I ran seven `fabro_seeds_search` queries (fabro_run_search, preflight legend, header stage-journal, lints-the-whole-repo-twice, glob_filter, fs_hide glob, firehose) — **all failed with 401 Unauthorized**, so live dedup was impossible. Substitute evidence: the planner's own `seeds ready --assignee loop --limit 200` output captured in this run's events (19:00:52, 64 open seeds with ids/titles), which I checked each recommendation against. "No existing seed" below means "not in that snapshot" — blocked or reassigned seeds outside the ready view were not inspectable.

Run baseline (from run events / conclusion): 191.7s wall, $0.170 total (planner $0.071 / implementer $0.061 / reviewer $0.039), zero retries, all gates first-pass green. The graph's happy path is clean; the waste is in repeated lint work, a missing tool, and planner context bulk.

---

**1. Deduplicate the gate's lint tiers — seed fabro-0d77 (open, Medium).**
Evidence: tester output shows `lint-nu: 66 scripts` and the full 25-warning prompt-lint block **twice verbatim** (validate-workflows section, then battery-runner section), and the implementer's recorded checks ran them a third time; 12.8KB gate output, 21.1s tester wall.
Change: `.fabro/workflows/loop/scripts/loop-gate.nu` — run lint-nu/prompt-lint once (battery-runner) and drop the duplicated tiers from the validate-workflows invocation, per fabro-0d77.
Effect: tester ~21s → ~10s every run, gate output ~halves, which also shrinks the reviewer preamble (reviewer consumed 21.5k input tokens, much of it this output).

**2. Fix the never-injected planner tool / absorb the in-flight scan into preflight — new seed.**
Evidence: graph declares `x.fabro_tools="fabro_run_search"` on planner, run spec has `agent.fabro_tools: false`, the actual injected tool list (attractor.tools event) contains no fabro tool, and the planner journaled "fabro_run_search tool absent from planner toolset, journaling degraded in-flight arm" — the open-PR half of in-flight exclusion silently didn't run.
Change: either make the engine honor node-level `x.fabro_tools` over the run-level kill-switch, or (script-layer, ADR-0029-friendly) move the open-PR/non-terminal-run scan into `nu .fabro/workflows/develop/scripts/planner-preflight.nu`.
Effect: eliminates the duplicate-work risk of two runs landing the same seed class; removes the per-run degraded journal entry.
Justification: no seed in the ready snapshot covers tool wiring or preflight absorption (fabro-6b58 bounds the check, fabro-9372 is a different projection field).

**3. Stop feeding the planner the 64-seed firehose — seed fabro-c3b4 (open, Medium).**
Evidence: `seeds ready` returned 64 issues / 9.3KB with `stdout_truncated: true`; planner reasoning shows it re-derived priority ("Priority order unknown") that the preflight table had already ranked, across 9 messages / 52.8s inference — 42% of run cost to claim a comment-only seed.
Change: `.fabro/workflows/loop/prompts/planner.md` steps 1–2 — preflight's ranked candidate table becomes the primary ordering source; `seeds show` only the adjudicated top candidate; `seeds ready` demoted to degraded-mode fallback.
Effect: fewer planner turns per claim, directly cutting the run's largest inference block.

**4. Move the static preflight legend out of runtime context — new seed.**
Evidence: preflight@1's routing JSON is 3,127 bytes, of which the immutable `legend` advisory text is ~2.2KB; it was inlined verbatim into the planner's prompt (UserInput event) — identical text on every run, both lanes (script is shared with develop).
Change: `.fabro/workflows/develop/scripts/planner-preflight.nu` — keep `legend` on stdout only, drop it from `context_updates.output.preflight`, and document verdict semantics once in the planner prompts.
Effect: ~2KB less planner prompt per pass on every loop and develop run; candidates stand out instead of sitting under a wall of advisory prose.
Justification: nothing in the ready snapshot mentions the legend or preflight output size; fabro-c3b4 addresses candidate volume, not static payload.

**5. Port the fs_hide/glob_filter footgun lines to the loop planner prompt — extend scope of fabro-6997 / fabro-8d61 (both open, develop-scoped).**
Evidence: planner journal — "grep tool with glob_filter found nothing but glob+shell grep worked"; the planner's own `x.fs_hide=".fabro/**"` makes its file-tool probes no-ops on loop assets, so glob+grep calls were wasted before falling back to shell (this lane's every seed touches `.fabro/**`).
Change: add the two one-liners (fs_hide globs return empty → use shell `rg` for loop assets; glob_filter syntax) to `.fabro/workflows/loop/prompts/planner.md`.
Effect: 2–3 fewer no-op tool calls per planner pass on every loop-asset seed.
Justification if filed separately: both seeds name develop's prompt file; the loop prompt is a distinct file no open seed covers.

**6. Gate-check enumerated workflow lists in shared-script headers — new seed.**
Evidence: this run exists only because the stage-journal.nu header drifted ("four" vs six workflows, residual from fabro-4cc3); the implementer's journal proposes exactly this: "a lint rule that cross-checks enumerated workflow names in shared-script headers against actual references would catch the next drift at gate time."
Change: add a battery (registered in `.fabro/scripts/battery-runner.nu`, logic in `.fabro/scripts/prompt-lint.nu` or a new check): for shared `.fabro/scripts/*.nu` headers enumerating workflow names, diff against `rg -l <script> .fabro/workflows --glob workflow.toml`.
Effect: the next stale header goes RED deterministically at the tester instead of surviving to reviewer discretion and spawning a residual seed — kills this run's defect class.
Justification: no ready-list seed covers it (fabro-0d16 is an unrelated prompt-lint marker-drift fix).

---

Not worth acting on: `claim_check`'s "no range to scan — arm skipped (fail-open)" is benign on a comment-only diff; PR #429 auto-created and merged cleanly (19:03:32→19:03:52) — the closeout/PR/notify UX worked as designed.
