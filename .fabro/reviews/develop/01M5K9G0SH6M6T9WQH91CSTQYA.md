# Improve review — run 01M5K9G0SH6M6T9WQH91CSTQYA

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (~11.1 min, revisor pass — reason and cost in run detail)
- generated: 2026-10-23 05:05+0200 by revisor `fabro_ask`

---

## What this run actually did (grounding)

Run `01M5K9G0SH6M6T9WQH91CSTQYA` (loop lane, seed `fabro-85f6`, a **+4/−2 comment-only edit** to `.fabro/workflows/revisor/workflow.fabro`) succeeded in 11.1 min wall / $0.398 total. From run events: planner = 401.7s wall (399.8s inference, $0.237 — **60% of wall, 59% of cost**), implementer = 147.2s ($0.121), tester = 53.7s with 8.8KB output, evidence = 11.5KB output, reviewer = 25.6s. Tool errors occurred: `fabro_run_set_title` 3 calls/2 errors, `fabro_run_search` 2/1, `edit_file` 3/2 (all from stage tool stats in run events).

## Recommendations, by expected impact

**1. Stop the planner re-pulling the full seed firehose — seed `fabro-c3b4` (open)**
Evidence: the preflight node already returned the ranked top-5 candidates inline at 02:30:41, yet the planner's *first* shell call was `seeds ready --assignee loop --limit 200`, pulling 66 rows / 9.6KB it never needed (run events, planner pebble stream). Planner then spent 16 shell calls adjudicating.
Change: in `.fabro/workflows/loop/prompts/planner.md` step 1, make the preflight table the primary candidate list and demote `seeds ready` to a degraded/empty-table fallback, top-N only (per `fabro-c3b4`).
Effect: fewer planner turns and ~10KB less context in the stage that consumed 60% of run wall time and cost.

**2. Diff-scope the loop gate battery output — seed `fabro-a827` (open)**
Evidence: tester output was 8,848 bytes for a one-file comment diff; ~5KB of it is synthetic-fixture-id notes, 20 stale date-pin warnings, and routing-named schema warnings — none about the diff. `loop-gate.nu` line 93 runs `battery-runner.nu all` unconditionally and `check` (line 80) prints full stdout.
Change: in `.fabro/workflows/loop/scripts/loop-gate.nu`, filter battery-runner output to files in the run diff (pass the diff file list through), echoing only counts for out-of-diff surfaces.
Effect: tester output drops to diff-relevant lines; smaller reviewer/evidence payloads; red gates triage faster.

**3. Deterministically enforce the implementer pre-gate scoping rule — new seed**
Evidence: the planner prompt forbids implementer whole-repo lints ("the double battery run cost this lane 21.6s per run — run 01M550CAX"), yet evidence's recorded checks show `[by implementer] nu .fabro/scripts/prompt-lint.nu` (77 files) and `[by implementer] just lint-nu` (65 scripts) — and the tester then re-ran the identical battery at 02:39:53.
Change: teach `.fabro/workflows/develop/scripts/check-transcript.nu` to refuse whole-repo lint/battery commands when `--by implementer` (fail the probe with a pointer to the single-file probe rule).
*New-seed justification:* searches for "implementer lint battery" and "check-transcript" returned 0 tracker seeds; the prose rule exists only in planner.md and this run proves it doesn't hold.
Effect: eliminates the duplicate full-battery run every implement pass (~20–50s/pass plus duplicated output in evidence).

**4. Dedupe reviewer context — seed `fabro-17fb` (open)**
Evidence: the loop reviewer's preamble inlines the tester stage section (8.8KB, via `command.output` in `x.preamble_allow_keys` and tester not in `x.preamble_stages_ignore`) *and* evidence's recorded checks (11.5KB), which contain the same prompt-lint run verbatim.
Change: in `.fabro/workflows/loop/workflow.fabro`, `reviewer` node — once evidence carries recorded checks, add `tester` to `x.preamble_stages_ignore` (mirroring `fabro-17fb` in the loop lane).
Effect: ~10KB less reviewer input per review; reviewer used 22.4k input tokens for a 2-tool review of a 6-line diff.

**5. State the 100-char title cap in the planner prompt — new seed**
Evidence: `fabro_run_set_title` 3 calls / 2 errors (stage tool stats); the cap lives only in the tool description, not in planner.md's RUN TITLE paragraph; the run carried the generic title "Maintain the loop's own machinery one seed per run" from 02:30:35 until the claim completed (~02:37).
Change: one clause in `.fabro/workflows/loop/prompts/planner.md` RUN TITLE paragraph: "≤100 characters — the tool hard-rejects longer."
*New-seed justification:* search "set_title title" returned 0 seeds; no existing seed covers the title-cap prompting gap.
Effect: title lands in one call, kanban/Slack UX correct for ~6 more minutes of the run.

**6. edit_file bare "not found" recurred — seed `fabro-e33d` (closed) + follow-up**
Evidence: implementer `edit_file` 3 calls / 2 errors; its journal painpoint (verbatim): the matcher rejected a byte-identical old_string twice and "should report WHY a match failed… instead of a bare 'not found'"; the implementer burned shell calls on `sed -n | cat -A` debugging.
Change: the loop-lane half of `fabro-e33d` — pin old_string sourcing (copy verbatim from `read_file` output, prefer ASCII punctuation in new_string) in `.fabro/workflows/loop/prompts/implementer.md`.
*New-seed justification for a follow-up:* `fabro-e33d` is closed yet the symptom reproduced in this run — the closest-match engine hint is still absent, so a reopen or follow-up seed is warranted.
Effect: fewer implementer retry/debug cycles (implementer inference was 137.8s; retries feed directly into it).

**7. Require English stage responses — seed `fabro-eaa0` (open)**
Evidence: implementer@1's entire closing response is in German ("Die Implementierung ist abgeschlossen…", ~1.5KB), which is the stage Output of record.
Change: add the one-line language requirement to `.fabro/workflows/loop/prompts/implementer.md` per `fabro-eaa0`.
Effect: stage outputs stay greppable/consistent for reviewers, journals, and downstream preambles.

**8. Sweep stale prompt-lint date pins — seed `fabro-801e` (open)**
Evidence: every prompt-lint invocation (implementer, tester, evidence — 3× this run) emitted 20 "date pin older than 45 days" warnings, including `loop-gate.nu`'s own `2026-04-14` pin (line 55); that's ~60 noise lines per run and it buries real warnings.
Change: execute `fabro-801e` — re-validate or retire each dated pin in the flagged files (`conductor/`, `develop/`, `loop/`, `revisor/` prompts + workflow.toml/.fabro).
Effect: gate noise drops materially, compounding rec 2's signal-to-noise win.

**9. Long-stage heartbeat — seed `fabro-b769` (open)**
Evidence: nothing user-visible happened between preflight completing (02:30:41) and planner finishing (02:37:24) — a 6.7-minute silent stretch that is 60% of the run (run event timestamps).
Change: adopt `fabro-b769` (heartbeat notification for stages exceeding a threshold), sized to this run's planner profile.
Effect: operators watching #dev-fabro see liveness during the run's dominant silent phase instead of assuming a stall.

**Searches run for dedup** (live tracker): `fabro-a827`, `fabro-17fb`, `fabro-c3b4`, `fabro-b769`, `fabro-801e`, `fabro-eaa0`, `edit_file`, `set_title title`, `implementer lint battery`. All cited seeds confirmed open except `fabro-e33d` (closed, recurrence documented above).

**Evidence gaps:** loop-gate.nu does not record per-tier timing, so I can't split the tester's 53.7s across validate/battery/cargo-nextest (the dot-snapshot `cargo nextest` tier did run for this comment-only graph touch); and I could not inspect the engine's native `edit_file` matcher (product code, out of this run's scope) to confirm whether `fabro-e33d`'s engine half ever landed.
