# Improve review — run 01M219SFJ6A75DS2BP77TBENQZ

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (2.9 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-08 20:09+0000 by revisor `fabro_ask`

---

All evidence below is from this run's event stream, stage journals, checkpoints, and workspace files. Context: the run was a clean first-pass success — seed `fabro-8d1c` (docs-only ADR-0019 extension), 2m55s wall / $0.133 total, planner $0.056 @ 43.5s, implementer $0.050 @ 45.8s, reviewer $0.027 @ 23.8s, deterministic stages 6.5s combined. The graph design fundamentals worked (path-scoped gate printed "no crates touched" in 5.6s, evidence reached the reviewer inline, journal contract 7/7). Recommendations, ordered by expected impact:

## 1. The in-flight PR guard is a silent no-op — enable `fabro_tools` for develop (config bug, recurring)
**What happened:** The planner node declares `fabro_tools="fabro_runs_list"` (`.fabro/workflows/develop/workflow.fabro:80`), but `agent.tools.available` for planner@1 (run events seq 27) lists no such tool, and run settings show `agent.fabro_tools=false`. The planner journaled it: *"fabro_runs_list tool was not available in this planner session, so the in-flight PR check ran with zero exclusions — degraded fail-open."* The prior run 01M2183254XQ3HJ45SR4P1Y2X6 documented the identical degradation (`.fabro/reviews/develop/01M2183254XQ3HJ45SR4P1Y2X6.md:24`) — so this is now at least two consecutive runs with the double-pick guard (fabro-06e0, built to prevent a repeat of fabro-22e4: identical fix re-implemented while PR #47 waited in gate) blind.
**Change:** Add the run-level `fabro_tools = true` to `.fabro/workflows/develop/workflow.toml` — exactly what `.fabro/workflows/revisor/workflow.toml:11` and conductor (line 18) already set. Develop is the only `fabro_tools`-declaring workflow whose TOML omits it.
**Effect:** The guard actually excludes seeds with open run-linked PRs; eliminates the duplicate-implementation failure class (a duplicate costs a full ~$0.13/3-min run plus a redundant PR) instead of hoping no two runs collide.

## 2. Planner `sd ready` firehose — 23.8 KB / 177 rows into a 24 KB-budget context (cost)
**What happened:** The planner's first call (seq 32–34) returned **23,844 bytes, 177 ready issues**. Conversation tokens jumped to 9,641 by round two; the planner needed 5 LLM rounds and became the run's most expensive stage ($0.056, 43.5s, 42% of run LLM cost) for what was a trivial claim — the top candidate was the first High line. Open seed fabro-c3b4 (top-N view) predicted this; this run is fresh confirmation.
**Change:** In `.fabro/workflows/develop/prompts/planner.md`, change the `sd ready` row to a bounded view (e.g. `sd ready --assignee fabro --limit 200 | head -25` — full listing only when the top doesn't decide).
**Effect:** Cuts ~6k tokens of dead listing from every planner pass; fewer tie-break rounds; expect the planner under ~30s/$0.03 on simple claims.

## 3. `edit_file` gives a bare error and the agent misdiagnosed it — add a closest-match hint (error handling)
**What happened:** The implementer's first edit failed with `old_string not found in file` (seq 92) on a string it reconstructed from `read_file`'s line-numbered rendering (which shows a gutter + shifted indent; the file has 3-space indents, the reconstructed `old_string` had 4). It then burned a `cat -A` diagnostic call (seq 96–97) and still journaled it as a *"transient matcher hiccup"* — a wrong lesson recorded durably. The error string is a bare `Err` at `lib/components/fabro-agent/src/tools.rs:234`.
**Change:** (a) Engine: at `tools.rs:234`, include the line number of the closest fuzzy match / first diverging whitespace in the error; (b) prompt: one line in `.fabro/workflows/develop/prompts/implementer.md` — "build `old_string` from raw `sed -n` output, never from the line-numbered `read_file` rendering."
**Effect:** Removes the diagnostic round + one LLM round per mismatch, and prevents false "transient" conclusions from being journaled as lore.

## 4. Pipeline progress header undercounts in every stage prompt (UX)
**What happened:** The implementer's prompt read "Pipeline progress: 0 of 7 stages completed" when `start` + `planner` were done; the reviewer's read "2 of 7" when five nodes were complete (run events seq 73, 150). Matches open seeds fabro-45bf / fabro-a0e3 (unique-node denominator) — this run confirms it's still live in the current workflow version.
**Change:** Engine preamble renderer — count unique completed nodes, not per-thread visits.
**Effect:** Stage prompts stop understating progress; removes a recurring source of agent confusion about where it is in the loop (and the stale duplicate seeds can close).

## 5. Implementer's unconditional `sd show` re-fetch (tool economy)
**What happened:** The brief in context was a complete, faithful bulleted distillation, yet prompt step 1 mandates re-reading the seed — the implementer spent a shell round re-fetching `fabro-8d1c` (seq 83, ~2 KB output that matched the brief verbatim). Open seeds fabro-4881/fabro-a67f proposed gating this.
**Change:** `.fabro/workflows/develop/prompts/implementer.md` step 1: re-fetch only when the brief is thin or ambiguous; a bulleted brief from the planner is authoritative.
**Effect:** One fewer tool round + shorter context per implementer pass (~5s, small $).

## 6. Metadata snapshots run on every checkpoint, including no-op ones (~5% of wall)
**What happened:** Init snapshot 2,241 ms (seq 16), post-planner 1,930 ms (seq 68), and one per checkpoint across tester/evidence/reviewer/closeout — ~8–10s of the 175s wall on a run whose deterministic tail changed almost nothing between snapshots. Open seed fabro-c2ca (skip non-agent stages) / fabro-cf03 (async) cover this.
**Change:** Engine checkpoint pipeline — snapshot only after agent stages or on branch-push points.
**Effect:** ~5% wall-time reduction on lean runs; compounding across the one-seed-per-run cadence.

**Not worth changing based on this run:** the tester timeout (gate finished in 5.6s — path scoping already works), the reviewer's `preamble_inline_max_kb=16` (capture arrived inline, no blob detour, 23.8s approval), and reasoning_effort=low everywhere (764 reasoning tokens total across the run — already minimal).
