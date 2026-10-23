# Improve review — run 01M5KJ2NWJP1TM6JZZKQ2KHNX5

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (90.7s wall, revisor pass — reason and cost in run detail)
- generated: 2026-10-23 07:41+0200 by revisor `fabro_ask`

---

## What this run actually was

From run events and the run projection: this was a **bookkeeping-only run**. `tracker_guard` requeued stale claim `fabro-30a5` (left `in_progress` >6h by run 01M5KGBR…, PR #480 "no changes"); `preflight` flagged it `duplicate` (landed sha `a3f0834e9` = this run's own base); the planner spent **59.7s / 23,527 input tokens / $0.0675** adjudicating and closing it via the two-branch rule, then exited "Already landed". No implementer/tester/reviewer ran; final diff is 2 files +6/−1 (tracker + journal only); PR #482 was opened post-terminal (glm-4.7, 68s) for that diff. The planner's own journal painpoint names the cause. I also read the shared closeout script to confirm the mechanism.

## Recommendations (by expected impact)

**1. Give closeout a satisfied-close arm so "no changes" seeds stop parking forever** *(error handling / graph design)*
- Evidence: run 01M5KGBR… finished closeout green with `fabro-30a5` still open (planner painpoint, from run events); root cause is in workspace file `.fabro/workflows/develop/scripts/closeout.nu` lines 531–540 — the fabro-02c4 PARK gate refuses to close whenever the claim-anchored diff (tracker/journal excluded) shows no implementing change, and `demand-visible` returns false on an **empty patch by construction** (line 159). A no-change seed can therefore *never* close at closeout; it waits 6h for requeue, then costs a full extra run (this one: 90.7s wall, $0.067, + PR #482) to close.
- Change: in `closeout.nu`, close instead of PARK when the patch is empty/tracker-only **and** a reviewer-approved or planner-recorded verification marker exists (e.g. a `verified-satisfied:` journal marker, mirroring the existing `exemption:` marker contract in the same file); wire the marker emission into `.fabro/workflows/loop/prompts/planner.md` branch (b).
- Effect: eliminates one wasted full run + one human-merged PR per no-change seed; this exact shape has now happened twice (#480 → this run).
- **New-item justification:** searches `closeout verification-only`, `no changes close`, `verification-only`, `park` found no open seed covering close-semantics for no-change seeds — open `fabro-9e2c` only *surfaces* the park state, it doesn't fix the park-or-close decision.

**2. Don't open a human-merge PR for tracker/journal-only diffs** *(UX / publish path)*
- Evidence: this run's entire diff is `.seeds/issues.jsonl` + journal (+6/−1, from checkpoints), yet PR #482 was created (spec has `auto_merge: false`); until it merges, `fabro-30a5`'s close is invisible at base `a3f0834e9`, so the next scheduled run will re-encounter it as a duplicate — the same TOCTOU class as (closed) `fabro-006d`.
- Change: gate PR creation on diff content in the publish step (workflow.toml `pull_request` config / engine): skip or auto-merge when the diff touches only `.seeds/**` and `.fabro/journal/**`.
- Effect: no human merge for bookkeeping runs; kills the duplicate-work race window.
- **New-item justification:** searches `pull request tracker`, `bookkeeping`, `auto-merge` — open `fabro-b1d3` only annotates the Slack payload, open `fabro-2d0f` is about missing auto-merge requests on green PRs; neither covers suppressing/auto-merging tracker-only PRs.

**3. Ship the top-N ready view — stop inlining the 65-row firehose** *(prompting / tool usage)*
- Evidence: the planner's first shell call `seeds ready --assignee loop --limit 200` returned 65 rows / 9.4KB with truncated titles; the decision used only row 1, and preflight's ranked 5-candidate table was already in context (from run events, planner transcript).
- Change: implement open item **`fabro-c3b4`** ("Use a top-N sd ready view in the planner instead of the full firehose", @loop) — edit the seeds command table in `.fabro/workflows/loop/prompts/project-facts.md` to a top-N view.
- Effect: ~2.4K fewer conversation tokens per planner pass, faster adjudication, no misreads of truncated titles.

**4. Strip the static preflight legend and dedupe its double carriage** *(tool usage / context hygiene)*
- Evidence: preflight stdout is 3.7KB, ~half of it the byte-identical `legend` object re-emitted every run, and the same JSON reaches the planner twice (stage-transcript "Completed stages" output **and** the `output.preflight` context key via `x.context_allow_keys` on the preflight node — both visible in the planner UserInput event).
- Change: in `.fabro/workflows/develop/scripts/planner-preflight.nu`, drop the legend from stdout (move its text once into `.fabro/workflows/loop/prompts/planner.md`); emit only the candidates array.
- Effect: ~2KB less context per planner pass and one fewer copy of the verdict table.
- **New-item justification:** `legend` search shows the legend was *added* by closed `fabro-ead4`; no open item covers removing its per-run inline cost (`fabro-17fb` is the analogous but reviewer/tester-scoped dedupe).

**5. Extend the English-response rule to the planner prompt** *(prompting / UX)*
- Evidence: this run's planner stage response is German prose wrapping the routing JSON; it feeds the PR-body model and human review (from run events, planner output).
- Change: one line in `.fabro/workflows/loop/prompts/planner.md` (and reviewer.md), folding it into open item **`fabro-eaa0`** ("Require English responses in implementer prompt", @loop) by widening its scope to the loop lane's agent prompts.
- Effect: consistent language in stage outputs and PR bodies.

**6. (Lower, verify first) Scope the planner's memory load** *(cost)*
- Evidence: `MemoryLoaded` shows AGENTS.md at 28,723 bytes → 7,346 tokens = 31% of the planner's 23.5K input (context breakdown, from run events), for a pass that needed only tracker + preflight data; I could not verify from this run's transcript that any of it was used.
- Change: lane/node-scoped memory or a trimmed AGENTS.md for the planner node.
- Effect: ~30% input-token cut per planner pass.
- **New-item justification:** `AGENTS.md` searches (`AGENTS.md`, `memory AGENTS`) return only content-correctness items (`fabro-3b12`, `fabro-cc23`, `fabro-f37b`); none covers memory sizing per node.

**Not inspected:** run 01M5KGBR…'s own event stream (out of this run's scope) — the PARK reconstruction rests on this run's `tracker_guard` requeue output, the planner painpoint, and the closeout script's logic.
