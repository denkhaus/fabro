# Improve review — run 01M5K7S2ZA7Y9ZS6SD5S9QED92

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (4m45s, $0.24, terminal by 2026-10-23 04:21 Berlin, revisor pass — reason and cost in run detail)
- generated: 2026-10-23 04:34+0200 by revisor `fabro_ask`

---

All evidence gathered. Here are the recommendations, each grounded in this run's events, gated against the live tracker.

## What this run actually was

A 4m45s, $0.24 run (planner: 255s of 285s wall, 39.5K input / 5.7K reasoning tokens, 26 tool calls) whose entire output was re-adjudicating and closing **fabro-30ae** — a seed the *previous* run (01M5K62554W7YCVZ7W4AT8YYWK) had already verified and landed as commit `c3eea6563`, but left open. Final diff: 2 files, +6/−1 (tracker bookkeeping only). Sources: run events (tracker_guard/preflight/planner transcripts, conclusion timings).

## Recommendations, by expected impact

**1. Close the closeout gap that caused this whole run** *(error handling)*
- **Change:** `.fabro/workflows/develop/scripts/closeout.nu` (the `closeout` node, shared `--lane loop`): close the run's claimed seed — recoverable from the stage journal — whenever reviewer status is `succeeded`, **including verification-only approvals**, not only when a specific context key is present. This is verbatim the painpoint the planner journaled this run.
- **Effect:** prior run's closeout reported success but left fabro-30ae `in_progress`; tracker_guard requeued it 25 min later (tracker_guard output: `"requeued":["fabro-30ae"]`), and this run burned 255s + $0.24 re-adjudicating. Fix eliminates that entire class of follow-up run.
- **Seed:** NEW — searches run: `closeout` (13 seeds; closest fabro-02c4 is closed and about demand-visibility discipline), `verification close seed` (fabro-aa3d/fabro-d183, no), `commit message seed reference` (0). *Justification: no open seed covers closeout closing the claimed seed on reviewer-succeeded; this run's journal painpoint has not been filed yet.*

**2. Make already-landed verifications machine-detectable in preflight** *(graph design)*
- **Change:** `.fabro/workflows/develop/scripts/planner-preflight.nu`: extend the duplicate/already-landed arm to also scan `.fabro/journal/<run_id>.jsonl` for the candidate seed id when no commit message references it, and emit it as an advisory verdict.
- **Effect:** preflight reported fabro-30ae `"verdict":"clean"` while `c3eea6563` sat in base — because that commit never referenced the seed id. The planner then re-derived the verification itself (re-reading petri `stage.rs:201-211` in the worktree, per its own response). A journal pointer would have routed "Already landed" at the guard stage for zero LLM tokens instead of a 253s inference stage.
- **Seed:** NEW — searches run: `preflight landed duplicate` (only closed fabro-7280, a different bug: wrong base branch), `journal duplicate already-landed` (0). *Justification: closed fabro-d183 covered the already-landed routing exit, not detection sources; nothing open covers journal-derived duplicate detection.*

**3. Kill the 67-row `seeds ready` firehose** *(tool efficiency / cost)*
- **Change:** `.fabro/workflows/loop/prompts/planner.md` (candidate-source step): adjudicate from the preflight's inline 5-candidate table plus a top-N `seeds ready` view, instead of `--limit 200` pulling all 67 rows (~9.8KB) into context (run event seq 95).
- **Effect:** smaller planner context (conversation was 27.7K tokens this run), fewer distractions, faster single-seed selection every run.
- **Seed:** **fabro-c3b4** (open, @loop — "Use a top-N sd ready view in the planner instead of the full firehose").

**4. State the 100-char title cap in the planner prompt** *(prompting)*
- **Change:** `.fabro/workflows/loop/prompts/planner.md`, ADR-0030 title paragraph: replace "the seed's demand in your own few words" with an explicit "≤ 100 characters" instruction. The prompt's open-ended phrasing produced 139- and 121-char titles; `fabro_run_set_title` rejected both with "run title must be at most 100 characters" (event details seq 273, 280) before the 100-char third attempt succeeded.
- **Effect:** saves 2 failed calls + ~12s per claiming run; title lands first try.
- **Seed:** **fabro-b529** (open — same 100-char cap rejection mechanism, filed from revisor select; widen its fix to the loop planner instruction).

**5. Add a schema-valid `fabro_run_search` example to the in-flight check** *(prompting / tool usage)*
- **Change:** `.fabro/workflows/loop/prompts/planner.md`, step 5: add one exact example call with valid run-status vocabulary (or "omit status"). The planner passed seeds-tracker statuses — `unknown run status 'queued'`, then `unknown run status 'in_progress'` (event details seq 134, 143) — and only succeeded by dropping the filter, which returned 20 unfiltered runs.
- **Effect:** the mandated in-flight exclusion check works on the first call instead of the third; fewer wasted round-trips in every planner visit.
- **Seed:** **fabro-f18a** (open, @loop — "Lint workflow-prompt tool-call JSON examples against the live tool schemas").

**6. Add a long-stage heartbeat notification** *(UX)*
- **Change:** notification settings / hook scripts (`.fabro/scripts/`): emit a heartbeat when a stage exceeds a few minutes. Notifications currently fire only on `run.completed`/`run.failed` (run spec); the planner ran 255s — 89% of the run — completely silent.
- **Effect:** an operator watching #dev-fabro can distinguish "planner working" from "planner stuck" well before the 33m stall watchdog.
- **Seed:** **fabro-b769** (open, @loop — "Add a long-stage heartbeat notification for run visibility").

**7. Extend the English-response rule to the planner** *(prompting)*
- **Change:** `.fabro/workflows/loop/prompts/planner.md`: add the English-only response rule. The planner's routing response this run embedded a German parenthetical ("Zeiten: Claim-Run des Vorrunds 23.10., 03:33 MESZ…"), which flows into run titles/PRs and downstream preambles.
- **Effect:** consistent, searchable run records; no mixed-language artifacts in the kanban/PR surface.
- **Seed:** **fabro-eaa0** (open, @loop — "Require English responses in implementer prompt"; widen scope to planner.md).

## Not separately recommended

The graph's routing itself worked exactly as designed (tracker_guard → preflight → planner → "Already landed" → exit, 4 of 10 stages, no retries, no gate cycles) — the waste was upstream data quality (items 1–2), not graph shape. Dedup searches run, per the tracker: `closeout`, `preflight landed duplicate`, `commit message seed reference`, `title`, `verification close seed`, `journal duplicate already-landed`.
