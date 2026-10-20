# Improve review — run 01M5CN9EC11K39YCG4HCHDJ6NE

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (497s, revisor pass — reason and cost in run detail)
- generated: 2026-10-20 13:24+0000 by revisor `fabro_ask`

---

All recommendations below are grounded in this run's own record — stage timings/cost from the run conclusion, tool-call stats from the stage projections, and the stage transcripts/journal via `fabro_run_events`. Run shape: succeeded, 10/10 stages, 0 retries, gate green first pass; wall 497 s, cost $0.473, of which the implementer alone was 320.9 s / $0.312 (66%).

## Recommendations, by expected impact

**1. Enforce the implementer pre-gate — it ran the whole-repo battery its own brief forbade.**
- What happened: the planner's brief prescribed "single-file exact-string greps only … whole-repo lint and battery tiers belong to the tester" (planner stage output), yet the implementer's verification report shows it ran `just lint-nu` (67 scripts) and `nu .fabro/scripts/prompt-lint.nu` (81 files) through the wrapper — both then re-run by the tester ~90 s later (both outputs visible in the evidence `recorded checks` and tester stage output). This is the exact double-battery the rule was written to stop (run 01M550CAX, 21.6 s/run, cited in the develop planner prompt), and the reviewer did not flag it.
- Change: `.fabro/workflows/loop/prompts/implementer.md` — add the explicit prohibition (the develop planner prompt carries it; the loop implementer prompt does not) and one line in `loop/prompts/reviewer.md` making an implementer-stage whole-repo lint run a reportable deviation.
- Expected effect: removes a redundant ~15 s battery run plus its transcript bloat per implement pass, and restores the contract the reviewer can audit.
- Seed: none found (searches: "implementer battery lint whole-repo tester", "battery double run cost" → 0 matches). **New-seed justification: the existing rule lives only in the develop planner's brief-side prose; no open seed covers porting/enforcing it in the loop implementer+reviewer prompts.**

**2. Collapse prompt-lint's advisory flood — it dominated both the gate output and the evidence capture.**
- What happened: prompt-lint emitted 35 warnings + ~26 notes (synthetic-fixture-id lists, 17 date-pin warnings, 20 duplicated "routing-named property" warnings across 4 schemas) in the tester output (8.7 KB, mostly this noise), and the same output again inside the evidence capture's `recorded checks` (38 KB capture) that the reviewer must read.
- Change: `.fabro/scripts/prompt-lint.nu` — aggregate repeats ("date-pin warnings: 17 files", "routing-named props: 4 schemas × 5 props") or demote date-pin/routing-named to a `--strict` tier; keep the per-file lines only for new/unexpected warning classes.
- Expected effect: tester output and evidence captures shrink several-fold; real regressions stop being buried; cheaper reviewer context (29.2 K input tokens this run).
- Seed: none found (searches: "prompt-lint warnings date pin", "lint noise output", "date pin" → 0 relevant). **New-seed justification: fabro-0d16 covers marker-name drift, not warning volume/aggregation — no open seed addresses lint-noise volume.**

**3. Planner tracker call efficiency — cite fabro-c3b4.**
- What happened: `seeds ready --assignee loop --limit 200` returned all 58 ready seeds (~8.4 KB) into planner context (planner transcript, seq 75–79) to pick one; planner was 93.5 s / $0.108, the run's second-costliest stage.
- Change: covered by **open seed fabro-c3b4** ("Use a top-N sd ready view in the planner instead of the full firehose") — implement it in `preflight`/`planner-preflight.nu` candidates output plus the planner prompt.
- Expected effect: smaller planner context, fewer wasted turns adjudicating 57 irrelevant rows.

**4. Title-cap retry friction — cite fabro-b529.**
- What happened: planner tool stats show `fabro_run_set_title` 3 calls / 2 errors — two 105-char titles rejected before one fit; the planner journaled this exact painpoint ("error says only 'at most 100 characters' with no length shown"). `fabro_run_search` also went 3 calls / 2 errors in the planner (error text not retrievable from the events I inspected).
- Change: covered by **open seed fabro-b529** (100-char cap too tight, rejected twice in revisor too); fold in the error-ergonomics half — engine tool error should echo submitted length and over-by-N, and the planner prompt (`loop/prompts/planner.md` + develop twin) should state the cap upfront.
- Expected effect: eliminates 1–2 wasted tool round-trips per claim pass across all lanes.

**5. Fix the check-transcript bare-literal parse trap — cost this run its only failed tool call.**
- What happened: the implementer's first wrapped probe died at nu parse time (`...command: string` rejects bare `true`/`false` as boolean literals, `nu::parser::parse_mismatch` before the wrapper body runs) — journaled as a painpoint naming `.fabro/workflows/develop/scripts/check-transcript.nu` line 67; it was 1 of only 2 errored tool calls in the whole run, and it left exactly the kind of misleading exit-1 transcript record this run's seed (fabro-16a7) existed to disambiguate.
- Change: `check-transcript.nu` — either type the rest param looser and stringify, or add a header-comment + prompt line (both lanes' implementer prompts, next to the NESTED-SHELL PROBES guidance) that check commands must not be bare nu literals (`sh -c 'exit 0'` style).
- Expected effect: no more parse-class exit-1 records from well-formed checks; fewer first-attempt/re-run pairs in transcripts.
- Seed: only closed fabro-e32c (nested quoting — different failure class). **New-seed justification: no open seed covers the boolean-literal parse-mismatch class; it was first observed and journaled in this run.**

**6. Render very long diff lines readably in the evidence capture.**
- What happened: the reviewer's journal observation: the capture truncated the loop-reviewer prompt hunk mid-line (the changed lines are 1,700+ chars single-line), forcing an extra `read_file`/`grep` to confirm the clause landed — the only tool spend in the reviewer stage (2 calls).
- Change: `evidence.nu` diff rendering — cap or fold individual lines past ~200 chars with an explicit `long line elided — use read_file` marker, instead of cutting mid-hunk.
- Expected effect: reviewer verifies prompt-file changes from context; no rescue reads; slightly lower review cost ($0.053, 30.7 s today).
- Seed: adjacent open seeds fabro-9a43 (reviewer-side read_file targeting) and fabro-eed2 (blob-ref paragraph marker) cover the reviewer's coping behavior, not the capture-side rendering. **New-seed justification: no open seed covers the evidence.nu long-line diff-rendering fix.**

**7. Long-stage visibility — cite fabro-b769.**
- What happened: the implementer stage ran 5 m 21 s — 65 % of the run's wall time — with no intermediate user-visible signal (all stage events are completion-time; the Slack notification only fires on run.completed/run.failed per run settings).
- Change: covered by **open seed fabro-b769** ("Add a long-stage heartbeat notification for run visibility").
- Expected effect: an operator watching `#dev-fabro` sees progress during long agent stages instead of a 5-minute silence.

Not inspectable: the exact error text of the two failed `fabro_run_search` calls (event search returned the surrounding records truncated, not the error payloads); if that friction matters, it would need a dedicated look. Everything else above is backed by this run's stage outputs, tool stats, timing, and cost records.
