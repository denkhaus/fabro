# Improve review — run 01M2W2CMK2SP2WK85FFGJT85CA

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (6.1 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-19 05:40+0000 by revisor `fabro_ask`

---

All grounding below is from this run's events (stage transcripts, journal records, timings, cost) for run `01M2W2CMK2SP2WK85FFGJT85CA` (seed `fabro-491b`, PR #266, 5m47s active, $0.489 total; planner $0.257/53%, implementer $0.188/38%, reviewer $0.045). Open-seed status was verified against the live `sd ready` output captured in this run's planner transcript (event seq 39).

## Recommendations, by expected impact

**1. Stop line-truncating the evidence capture in the reviewer preamble.**
- What happened: the evidence capture was 9,361 bytes (~300 diff lines). The reviewer's stage prompt rendered it as "(113 lines omitted)" — the omitted head contained exactly the integrity header and the `rotate-review.nu` seed-work diff the design front-loads. The reviewer recovered only by reading the evidence commit's tree directly via 3 git shell calls (reviewer journal painpoint + observation, run events seq 328–360). A less capable pass would have routed "Verification blocked" and burned a re-capture cycle.
- Change: reviewer node in `.fabro/workflows/develop/workflow.fabro` — raise `preamble_output_max_lines` from 200 to ~400 (the capture is well under the 16 KB `preamble_inline_max_kb`, so the byte cap is not the binding constraint; the line cap is).
- Expected effect: first-pass approvals judge from the capture as designed; no git-reconstruction detour and no risk of a wasted verification-blocked cycle on every run whose diff has many short lines.
- New-seed justification: no existing seed covers the reviewer's *line*-cap truncation — open fabro-cf3e is the KB axis (16→32) and fabro-9837 is the blob-marker axis; this run proves the 200-line cap is the one that bit.

**2. Make the planner preflight skip upstream-blocked seeds mechanically.**
- What happened: `fabro-af22` (High) tops `sd ready` but is unactionable — its fix sits in open upstream PR fabro-sh/fabro#784. This run's planner burned 5 tool calls adjudicating it (`sd show`, two git grep rounds, an `rg` for `find_skill_references`, and a `web_fetch` of the full GitHub PR page — 13.4 KB of HTML) before skipping (events seq 43–71). The planner journal flags that this will recur "every run" because the tracker cannot express "waiting on upstream".
- Change: `.fabro/workflows/develop/scripts/planner-preflight.nu` — add a verdict arm that reads a seed body's "UPSTREAM PR OPEN / close when #N merges" status block and emits `verdict: "blocked_upstream"` (excluded from candidates) instead of `clean`.
- Expected effect: removes ~50 s of planner wall and ~12 K tokens of context per run for as long as #784 stays open, and stops the top-of-list candidate from being re-adjudicated by every subsequent run.
- New-seed justification: no open seed covers upstream-blocked park semantics — fabro-d9f7 is stale *in_progress* requeue, fabro-0195 is automations config; neither touches the ready-list exclusion mechanism.

**3. Replace the planner's 200-seed `sd ready` firehose with a top-N view — seed fabro-c3b4 (open).**
- What happened: the planner ran `sd ready --assignee fabro --limit 200` and pulled 28,763 bytes / 200 seed lines into context (event seq 38), yet only the top ~5 candidates mattered — the preflight had already returned verdicts for exactly those 5. Planner input was 64,036 tokens, the run's costliest stage ($0.257, 53%).
- Change: `.fabro/workflows/develop/prompts/planner.md` step 1 + the sd table in `project-facts.md` — list top-N (e.g. `--limit 10`) on first call; the preflight table remains the authoritative candidate scan, with `--limit 200` demoted to the fallback when the top-N is exhausted or all flagged.
- Expected effect: cuts ~7–8 K tokens of planner context every run; on this run that is roughly a 10–15% planner-cost reduction and one less scroll of noise the model must ignore.

**4. Batch planner reconnaissance into chained shell calls — seed fabro-55a7 (open).**
- What happened: the planner made 16 sequential shell calls, each costing a 5–6 s LLM round-trip (seq 37–97: `sd ready` → `sd show af22` → git fetch/grep → rg → `sd show 491b` → rg → grep → two `sed` reads). Planner timing: 137.1 s inference vs 12.5 s tool time; 150 s wall = 46% of the run's active wall.
- Change: `.fabro/workflows/develop/prompts/planner.md` — make step 1–3 reconnaissance one or two chained calls (`sd ready … ; sd show <top-1> ; sd show <top-2>` in one shell invocation), reusing the pattern this run's own planner already used ad-hoc for the af22 git checks.
- Expected effect: ~8–10 fewer round-trips per planning lap → 40–60 s off planner wall time on every run.

**5. Chain the implementer's fixture smoke and verification rounds — seed fabro-866a (open).**
- What happened: the implementer ran 19 shell calls with tool_time of only 3.35 s against 132.7 s inference (136 s wall); the three fixture cases of `rotate-review.nu` were separate invocations, and the transcript shows recon/edit/verify as single-purpose calls.
- Change: `.fabro/workflows/develop/prompts/implementer.md` step 4 (verify section) — require the fixture smoke cases and the grep checks to run as one chained shell call per tier, mirroring the cost-tier rule that already exists for probes.
- Expected effect: 30–50 s less implementer wall per pass at unchanged verification coverage (the checks themselves cost milliseconds).

**6. Bound the `fabro_runs_list` in-flight call — seed fabro-6b58 (open).**
- What happened: the planner journal's second painpoint: the tool "returns 75 runs with full JSON per run — only status + PR state + goal matter." The tool already accepts `created_since` (per its own schema in the planner's tool list), but the prompt's step 4 instruction calls it unbounded, so the whole projection landed in planner context.
- Change: `.fabro/workflows/develop/prompts/planner.md` step 4 — mandate `created_since` (~48 h window) plus self-exclusion on every `fabro_runs_list` call.
- Expected effect: shrinks a recurring multi-KB planner input; complements fabro-9372 (exposing `current_seed_id` in the same projection) by making the payload small enough to matter.

**7. Put the per-stage cost/time table and painpoint digest in the PR body — seed fabro-1409 (open).**
- What happened: this run produced PR #266 with a clean conclusion block (per-stage wall/inference/tool times, $0.489 cost) and three actionable journal painpoints (planner ×2, reviewer ×1) — all of it visible only by digging through run events or `.fabro/journal/…jsonl`. Nothing surfaces in the place the user actually looks.
- Change: PR postlude (run settings' PR-body generation) — append the conclusion stage table and the journal painpoint digest to the PR body, as fabro-1409 specifies.
- Expected effect: friction found by the loop reaches the user at review time without journal archaeology; this run's painpoints (items 2 and 6 above) would have been visible in PR #266 itself.

Not recommended as separate changes: the preflight's false `missing_file` anchor flags on `fabro-7611`/`fabro-60a0` (seen in this run's verdict table) are already covered by open seed fabro-7611, and the implementer's mtime fixture gotcha was already captured as mx-76bb49.

One limitation: I did not re-open `.fabro/workflows/develop/workflow.fabro` to confirm the current numeric value of `preamble_output_max_lines` in the workspace (the value 200 is from the run's registered graph version in run events, which is what this run executed).
