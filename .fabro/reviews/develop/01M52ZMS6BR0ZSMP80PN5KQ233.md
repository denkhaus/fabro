# Improve review — run 01M52ZMS6BR0ZSMP80PN5KQ233

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (10m05s, revisor pass — reason and cost in run detail)
- generated: 2026-10-16 19:37+0000 by revisor `fabro_ask`

---

All evidence is in hand: full stage transcripts, gate outputs, journal entries, timings/cost (from run events + run conclusion), plus the tracker and the exact code sites (from workspace files). Run baseline: green end-to-end, 10m05s wall, $0.61 total, 6 stage retries — every retry traces to fixable friction below.

## Recommendations, ordered by expected impact

**1. Fix the CREATE_INTENT verb-proximity regex — the claim-check bounce loop cost 2 extra planner visits this run.**
- What happened (from run events): `claim_check` failed 3× (visits 1–3) on seed fabro-828c because the cited to-be-created paths flagged `missing_file`. Visit 2 failed even after the planner added "(create)" wording, because `CREATE_INTENT` at `.fabro/workflows/develop/scripts/anchor_check.nu:205` is `\W*$`-anchored — "created: scripts/up.nu (create) and scripts/install-url.nu" matches the first path but not the second ("(create) and" puts word-chars in the gap). Only planner@4 (placing a verb directly before *every* path) passed. The bounces consumed planner@2 + @3 (37.2s + 30.3s ≈ 68s, ~$0.11 of the planner's $0.27 total) plus 4 checkpoint commits — ~11% of run wall for zero work product. The planner journaled this exact fix idea 3× (planner@2/@3/@4 painpoints).
- Change: in `creation-named?` (anchor_check.nu:205/235), also accept a literal `(create)`/`(new)` suffix token immediately *after* the path (or drop the `$` anchor for a bounded gap between verb and path), so preflight and claim-check agree on creation paths.
- Expected effect: creation-type seeds — the lane's most common class (this run's seed was one) — claim on planner visit 1; ~1 min and ~$0.10 saved per such run.
- Seed: **new-seed justification** — no open seed covers the match shape: fabro-f2eb (closed) only moved the window to the shared seam, fabro-e8ae covers extension-cap truncation in the same file, and neither addresses verb-proximity.

**2. Enable run-level fabro tools for the loop lane — the planner's declared tool never loads.**
- What happened (from planner@1/@4 journals and run spec): the graph declares `x.fabro_tools="fabro_run_search"` on the planner node, but the session carried no fabro tool ("fabro_run_search tool absent… in-flight exclusion degraded to preflight branch scan"), because the run spec shows `agent.fabro_tools: false` and `.fabro/workflows/loop/workflow.toml` has no `[run.agent]` section to enable registration.
- Change: add `[run.agent] fabro_tools = true` to `.fabro/workflows/loop/workflow.toml` (one line, mirroring the develop fix).
- Expected effect: the IN-FLIGHT EXCLUSION prompt arm actually executes instead of silently degrading every pass — right now duplicate-claim protection rests entirely on the preflight branch scan (fabro-8932/a111/4cc3 were excluded that way this run, a partial cover).
- Seed: **fabro-9b84** (open) — its evidence append names exactly this suppressor and calls the loop-workflow half of the enable an open overflow.

**3. Land the nushell-semantics skill — implementer probe thrash was the run's costliest stage.**
- What happened (from implementer@1 journal + recorded checks in the evidence capture): three probe failures preceded the green ones — a `complete ignore` parse error, a bare `^flock` that aborted the script with exit 99 before the ALARM branch (nu 0.115 semantics), and a contention probe whose background lock holder was killed by `complete`, causing the probe to run the **entire `just up` pipeline** (cold cargo compile ~38s, bun build, `build-image` failing on absent docker) inside the run worktree. The implementer stage: 325.7s / $0.282 — the largest single stage, with a visible slice being probe thrash.
- Change: land the skill from fabro-d19e carrying the two exact rules hit here (branch-on-exit-code requires `do { … } | complete`; background holders need `setsid nohup` to survive `complete`), plus a one-line rule in `.fabro/workflows/loop/prompts/implementer.md`: probes never invoke pipeline recipes (`just up`/`build-image`) — use synthetic lock holders.
- Expected effect: 6 probe attempts → ~2 on lock/external-class seeds; minutes and tool-cost off every scripts-conversion seed (a standing sprint axis).
- Seed: **fabro-d19e** (open).

**4. Deduplicate the lint tier inside one gate pass.**
- What happened (from tester@1 output): the loop gate's own output (12.3KB) runs lint-nu and prompt-lint **twice** — once inside `just validate-workflows` (loop-gate.nu:83) and again inside `battery-runner all` (loop-gate.nu:84) — including the ~40-line prompt-lint notes/warnings block both times.
- Change: in `.fabro/workflows/loop/scripts/loop-gate.nu` lines 83–84, keep one lint execution (validate-workflows' or battery-runner's) and invoke the other for its non-lint tiers only.
- Expected effect: gate output roughly halves every run; smaller evidence captures downstream (see #5).
- Seed: **new-seed justification** — fabro-b18c covers registry drift and fabro-53cc covers harness consolidation; neither covers double execution of the same lint within one gate pass.

**5. Stop the reviewer preamble truncating the evidence diff.**
- What happened (from reviewer@1 journal): "The evidence preamble truncation cut off mid-tracker-diff (the fabro-828c line)" — the 43.3KB evidence capture against `x.preamble_inline_max_kb=16` on the reviewer node; the reviewer fell back to the stat map and made 3 read_file calls to recover. It approved correctly this time, but that is a verification-gap risk, not a design win.
- Change: either raise the loop reviewer node's `x.preamble_inline_max_kb` 16 → 32 (the graph's `x.preamble_budget_kb=48` accommodates it), or have `evidence.nu` suppress the static prompt-lint notes/warnings block (identical ~40 lines every run) from recorded-check captures — the latter also compounds with #4.
- Expected effect: the reviewer sees the full seed diff inline; fewer recovery reads; lower risk of approving against an unseen hunk.
- Seed: **new-seed justification** — fabro-020b caps *loop-churn* diffs only (this run's churn was 0 and it still truncated); fabro-31f7's 2000-line raise (already in the graph) doesn't touch the 16KB inline cap.

Not inspected: the planner/reviewer prompt files themselves (`prompts/planner.md`, `prompts/reviewer.md`) — recommendations 3 and 5 name them for edits but I verified their current text only via the graph attributes and rendered preambles in run events, not by reading the files.
