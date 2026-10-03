# Improve review — run 01M2R6R6KZFHFE7NKZK9WFKKJT

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (5.6 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-17 17:41+0000 by revisor `fabro_ask`

---

All evidence gathered. Here are the recommendations, ordered by expected impact and grounded in this run's events, journals, timings, and costs (run totals: 5m15s wall, $0.4218; implementer $0.2633/62% of cost, planner $0.0920, reviewer $0.0666, tester 5.2s).

---

**1. Stop shipping unrunnable verification commands in planner briefs — fabro-4be6**
- **What happened (run events + implementer journal):** The brief's cheapest-first bullet prescribed `nu -c 'use .fabro/scripts/dup-run-check.nu *; ...'`. The script's defs carry no `export`, so the implementer burned probe rounds ("Command not found" for named, glob, and sibling import forms) before falling back to a standalone regex script and the fixtures battery. The implementer logged 25 shell calls with 5 errors and finished at 188s inference vs 5.3s tool time — $0.263 of the $0.422 run.
- **Change:** Implement fabro-4be6 in `.fabro/workflows/develop/prompts/planner.md` (step 6/7 area): every literal verification command in a brief must be dry-run once before the brief ships; for export-less nu scripts, the brief prescribes the fixtures battery, never a `use` import.
- **Expected effect:** Eliminates the failed-probe LLM rounds on the costliest stage; implementer laps start verifying instead of debugging the brief.

**2. Run loop-asset fixture batteries in the deterministic gate — new seed**
- **What happened (tester output + reviewer journal):** The tester's gate was lint-only (`lint-nu: 24 scripts … GATE GREEN`, 5.2s), so the *only* behavioral test of this seed — the dup-run-check fixtures battery — was executed twice by LLM agents: once by the implementer and again by the reviewer ("re-ran the full dup-run-check fixture battery (24/24 pass …) since the qualitygate only lints loop-asset scripts").
- **Change:** Add self-contained fixture batteries (exit-0, mktemp-scratch pattern like `.fabro/scripts/dup-run-check-fixtures.nu`) to the loop-asset branch of the qualitygate (`justfile` → `scripts/qualitygate.nu`).
- **Expected effect:** The tester deterministically proves classifier behavior; the reviewer stops re-running suites; a future regex regression goes gate-red instead of unnoticed.
- **New-seed justification:** No open seed runs fixture batteries in the gate — fabro-8a60 only checks that seed-spec-named loop paths *appear* in the diff; fabro-50f8 covers null-path exercises in edits, not gate execution.

**3. Fix the preflight type_mismatch before the next stale-tracker run — fabro-2ade**
- **What happened (run events + workspace file):** The preflight node succeeded this run (1.09s, 5×clean verdicts, exit 0) — but only because no candidate was classified landed. `.fabro/workflows/develop/scripts/planner-preflight.nu:135` still reads `mut closed = {seed: null, sha: null}`; the string-record assignment fires only on the duplicate/landed path, i.e. exactly the case where the node is supposed to save 93.6s/$0.143 (per the seed's basis run).
- **Change:** Apply fabro-2ade: initialize `closed` with empty strings (or a `let` record) and test non-empty at line 161.
- **Expected effect:** The fabro-a32f saving materializes on landed-candidate runs instead of a runtime crash on that path.

**4. Shrink the implementer prompt — fabro-7b2a**
- **What happened (stage usage):** The implementer consumed 35,636 input tokens to implement a ~1.7KB seed, with a ~40KB prompt plus the 25.7KB `AGENTS.md` memory (~6.6k tokens) re-read every lap — at `reasoning_effort="low"` this stage is still 62% of run cost.
- **Change:** Implement fabro-7b2a in `.fabro/workflows/develop/prompts/implementer.md`: cap the step-4 operative rule and move run-id citations to footnotes.
- **Expected effect:** Direct, recurring token reduction on the dominant cost stage.

**5. Bound the planner's two firehose inputs — fabro-c3b4 + fabro-6b58**
- **What happened (planner transcript, seq 37–47):** `sd ready --limit 200` poured 200 seeds / 28.9KB truncated stdout and the planner picked the first line anyway; `fabro_runs_list` returned 45 runs, consumed 6.8s of the planner's 7.4s tool time, plus one LLM round to conclude "only this run is non-terminal."
- **Change:** fabro-c3b4 (top-N priority-sorted `sd ready` view) and fabro-6b58 (`created_since` window + explicit self-exclusion) in `.fabro/workflows/develop/prompts/planner.md`.
- **Expected effect:** Smaller planner preamble, ~6s less tool time and one fewer LLM round per run.

**6. Heredoc-first nu checks for the reviewer — new seed**
- **What happened (reviewer journal, 2 of its 5 shell calls errored):** The reviewer hit the mx-d44874 quoting trap twice (bash ate `$`s under double quotes, then nu rejected `\s`) before a heredoc temp script worked, and journaled that the recorded convention should teach "prefer a temp .nu file via heredoc" as first choice.
- **Change:** One line in `.fabro/workflows/develop/prompts/reviewer.md` (tools section): multi-line/regex nu checks go through a heredoc temp `.nu` file first; amend the mx-d44874 expertise record via `ml record`.
- **Expected effect:** Removes the two errored shell calls and the retry reasoning per review lap.
- **New-seed justification:** fabro-2904 covers *prompt-embedded* nu snippets; no open seed covers reviewer-side runtime shell discipline or the mx-d44874 amendment.

**7. Document the ml record flag contract — fabro-96bd**
- **What happened (implementer journal):** `ml record --type failure` requires `--resolution`; the first attempt failed without it — one of the implementer's five errored calls.
- **Change:** Implement fabro-96bd in `.fabro/workflows/develop/prompts/implementer.md` (step 6 / lesson-capture section).
- **Expected effect:** First-attempt lesson capture; no wasted call per implementer pass.

**8. Prioritize the evidence tail-lines fix — fabro-meta-c9f2**
- **What happened (reviewer observation):** "Evidence capture's omitted 54 lines covered the diff head" — the same summary:high renderer truncation fabro-meta-c9f2 documents (its basis: 119 omitted lines hid the seed-work diff). This run got lucky ("all material hunks were visible").
- **Change:** Land fabro-meta-c9f2 so the full per-seed capture renders without omission.
- **Expected effect:** Removes the review-blind-spot risk that a material hunk lands in the omitted tail.

**Not recommended despite temptation:** closing fabro-2ade as stale — the clean-path success this run does *not* disprove it; the buggy assignment only executes on the duplicate-verdict path (verified in the workspace file at line 135).
