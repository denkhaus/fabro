# Improve review — run 01M550CAXV85JGT1XBPCDRE5DY

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (11m08s, revisor pass — reason and cost in run detail)
- generated: 2026-10-17 14:10+0000 by revisor `fabro_ask`

---

All recommendations below are grounded in this run's events (stage transcripts, journals, timings, cost) and cross-checked against the seed tracker at `.seeds/issues.jsonl`. Run baseline for impact math (from run events/conclusion): 11m08s wall, **$0.516 total cost**, of which the implementer stage was 6m46s and $0.365 (71%); gate green first pass, zero retries, seed fabro-899b closed, PR #423.

---

**1. Enable run-level fabro tools so the planner's in-flight check stops running degraded — seed `fabro-d62c` (open, priority 1, needs-user).**
What happened: the planner journaled *"in-flight check degraded: no fabro_run_search tool available in this planner environment; relied on preflight in_flight flags"*, even though the graph pins `x.fabro_tools="fabro_run_search"` on the planner node (from planner stage response/journal). The run spec shows `agent.fabro_tools: false` — the node pin names tools, the run-level bit enables them (per fabro-96c6, cited in the seed).
Change: add `[run.agent] fabro_tools = true` to `.fabro/workflows/loop/workflow.toml` (all other nodes already pin `x.fabro_tools=""`, so only the planner gains tools).
Expected effect: planner step 5 executes as written; the duplicate-claim class (seed fabro-d62c cites ~15 min / ~$0.73 per duplicate) becomes preventable in the loop lane instead of silently degraded.

**2. Delete the duplicated lint tiers from validate-workflows — seed `fabro-0d77` (open, assignee loop).**
What happened: the tester transcript shows `lint-nu: 65 scripts` and the entire ~40-line prompt-lint warning block executed **twice per gate pass** — once inside `just validate-workflows`, once inside battery-runner lint (from tester stage output, 12.8KB, largely verbatim duplication).
Change: delete the two embedded lint tiers from `scripts/validate-workflows.nu` (lines ~69–88 per the seed); battery-runner stays the single lint entry — no loop-gate.nu edit needed (lines 83–84 stay).
Expected effect: every loop-gate pass parses the repo's ~65 nu scripts and 78 prompt files once instead of twice — the 21.6s tester stage and its log roughly halve, and real warnings stop appearing as duplicate noise that reviewers must skim twice.

**3. Add a nushell-scripts skill to cut implementer debug cycles — seed `fabro-d19e` (open).**
What happened: the implementer journaled a painpoint that nu string-interpolation footguns (`$("\("...")"` treating literal `(` as interpolation, `join '|'` mis-parsing in closures, parenthesized `++` chains) cost **~5 debug cycles** writing `.fabro/scripts/schema-parity-fixtures.nu`. That stage is the run's dominant cost: 6m46s wall, 96% inference, $0.365 of $0.516.
Change: create `.fabro/skills/nushell-scripts/` documenting the safe patterns the implementer already derived (single-quoted strings + `++` with simple variables, `str join`, zero literal parens inside interpolated strings); the implementer node's `x.tools` already includes `use_skill`, so no graph change.
Expected effect: directly attacks the largest time/cost bucket — each avoided debug cycle is one LLM round-trip on a 40K-token context; expect minutes and tens of cents saved on every nu-heavy loop run.

**4. Restore the clean develop fixture between parity-battery teeth — seed `fabro-2366` (filed by this run's closeout from the reviewer's finding).**
What happened: the reviewer approved but flagged that the `required-key drift` tooth mutates the temp *develop* fixture and never restores it, so teeth 4–6 run against a fixture still carrying `x-parity-teeth` — they RED, but don't isolate their divergence class (from reviewer stage response).
Change: in `.fabro/scripts/schema-parity-fixtures.nu` (~lines 180–190), re-save the clean develop record to `$dpath` before each subsequent tooth, mirroring the existing per-tooth loop-side re-save.
Expected effect: each of the six divergence classes fails independently, so a future RED names exactly one class in one read instead of requiring manual disentangling — cheap insurance on a battery that now guards both gates.

**5. Stop filing seeds with dead `tmp/` evidence anchors — new seed needed.**
*Justification: no open seed covers anchor hygiene at seed-filing time — fabro-d20f only governs how briefs format corrections after a dead path is detected.*
What happened: fabro-899b's body cited `tmp/arch-gate-sprint42/candidates.md`, which no longer exists; preflight flagged `missing_file`, the planner burned a tracker write (`seeds update --set-description`, re-emitting the full body) plus adjudication before claiming, and claim_check still passes it as an advisory. Fabro-0d77 carries the same dead anchor and will cost the next planner the same detour.
Change: in the arch-gate filing path (the sprint-42 report process / revisor that files candidates), require the measured evidence to be inlined into the seed body — never a `tmp/` path citation; alternatively make preflight's `missing_file` anchor auto-file a body-fix task at claim time.
Expected effect: eliminates a recurring pre-claim correction loop (one tracker write + one LLM adjudication per affected seed) and removes advisory noise from every downstream claim_check.

**6. Give the loop planner a top-N ready view instead of the 65-row firehose — seed `fabro-c3b4` covers the develop (`sd`) half; the loop-lane `seeds` mirror needs its scope extended (or a sibling filing).**
What happened: the planner's first shell call (`seeds ready --assignee loop --limit 200`) returned 65 rows / 9.5KB with `stdout_truncated: true` (from planner events, seq ~93–96), yet only the top 3 candidates were ever adjudicated; the planner runs at `reasoning_effort=low` with a 22.2K-token input budget.
Change: in `.fabro/workflows/loop/prompts/planner.md` (PROJECT_FACTS command table), make the default `seeds ready --assignee loop --limit 10`, widening to 200 only when surface-router skips exhaust the top slice.
Expected effect: smaller planner context and faster candidate adjudication on every loop run; the truncation flag stops firing on a list the planner mostly ignores.

**7. Scope implementer pre-gate checks to the diff, not the whole repo — new seed needed.**
*Justification: no open seed covers the implementer-vs-tester battery duplication; fabro-0d77 dedupes tiers inside the gate only.*
What happened: the implementer's recorded checks ran `just lint-nu` (65 scripts), full prompt-lint, full battery-runner, and run-scope — then the tester re-ran the identical battery ~3 minutes later (compare implementer summary against tester transcript). The brief's phrase "lint-nu clean on the new script" induced a repo-wide run because lint-nu has no single-file mode in that wording.
Change: in `.fabro/workflows/loop/prompts/planner.md` (Envelope note) prescribe single-file parse probes for new/edited scripts (e.g. `nu -c "source <file>"`) and leave whole-repo tiers exclusively to the tester node.
Expected effect: trims implementer tool calls and transcript size on the most expensive stage, and removes double execution of every lint/battery per run (measured here as the full 21.6s gate battery run twice).

---

Sources: stage timings/cost and transcripts from run events (`fabro_run_events`, stages `tester@1`, `planner@1`, `implementer@1`, `reviewer@1`, `closeout@1`); tool-availability and run-level `agent.fabro_tools: false` from the run spec (`fabro_run_get`); seed titles/statuses from `.seeds/issues.jsonl` (fabro-0d77 at line 887, fabro-2366 at line 890, fabro-d62c at line 876, fabro-d19e/fabro-c3b4 in the planner's `seeds ready` capture); gate structure from workspace file `.fabro/workflows/loop/scripts/loop-gate.nu`. One caveat: I could not inspect `scripts/validate-workflows.nu` line numbers directly — the ~69–88 range is as recorded in seed fabro-0d77's body.
