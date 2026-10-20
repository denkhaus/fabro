# Improve review — run 01M5BYTX2X8Q55PQ91RTHWAXB5

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (625.9s wall, revisor pass — reason and cost in run detail)
- generated: 2026-10-20 06:34+0000 by revisor `fabro_ask`

---

# Recommendations — grounded in run 01M5BYTX2X8Q55PQ91RTHWAXB5 (seed fabro-6b0f)

**Run baseline (from run events / conclusion):** 625.9s wall, 473.3s of it LLM inference, $0.558 total. planner 150.1s/$0.183, implementer two visits (184.8s/$0.173 + 143.4s/$0.162), tester ran twice (14.5s RED + 13.8s green), evidence 27,989-byte capture, reviewer 23.4s/$0.041 with `cache_read: 0`.

Dedup searches run before recommending: `seed-spec-paths`, `extractor`, `false red`, `provenance`, `journal`, `firehose`, `preflight`, `legend`, `heartbeat`, `nushell`, `lint`, `noise`, `date pin`, `fixtures`, `probe`, `sandbox`.

---

**1. Invert `spec-named-loop-paths` from deny-list drops to an editable-roots allow-list — highest impact: the false RED cost this run ~29% of its budget.**
Evidence: tester@1 failed with `seed-spec-paths: named .fabro/journal/01M59JSC3MKGRCH5PMR53XHADZ.jsonl — NOT in diff` — the seed's `Origin:`/`Basis:` provenance citation parsed as an edit mandate. The nominal fix had already landed in implementer@1; the entire implementer@2 lap (143.4s, $0.162) + a second tester pass existed only to fix the gate. This is the *second* piecemeal drop in that extractor (fabro-48f3 dropped directory tokens; this run dropped `.fabro/journal/`), and the implementer journal itself says: "when a tier's RED names a path no role may ever edit, treat it as an extractor bug."
Change: in `scripts/qualitygate.nu` (~line 346, `spec-named-loop-paths`), keep only paths inside the implementer's actual fs-write roots instead of filtering known-bad prefixes; add a fixture arm in `.fabro/scripts/seed-spec-paths-fixtures.nu` asserting a provenance-only body yields "tier idle".
Expected effect: eliminates the burned-implementer-lap failure class (~$0.16 + ~2.5 min per occurrence on every residual-citing seed).
Seed: **none open covers it** — only closed fabro-48f3 touches this tier (different drift). *New-seed justification: the journal-path fix landed inside this run as a stopgap; no open seed covers the general editable-roots inversion the journal painpoint asks for.*

**2. Ship fabro-c3b4 (top-N ready view) — the planner read a truncated 58-seed firehose.**
Evidence: from run events (planner tool call, `stdout_truncated: true`): `seeds ready --assignee loop --limit 200` returned 58 seeds / 8.4KB; the model then re-derived priority ordering in prose ("Medium first… prefer fewest blockers") over the whole dump. Planner is the most expensive non-implementer stage ($0.183, 150s, every run) and the dump grows with the backlog.
Change: `.fabro/workflows/develop/scripts/planner-preflight.nu` (shared by loop) emits a top-N candidates view; planner prompt (`.fabro/workflows/loop/prompts/planner.md`) consumes it instead of the raw list.
Expected effect: fewer planner turns/tokens per run and protection against silent truncation as the queue grows.
Seed: **fabro-c3b4** (open, @loop).

**3. Aggregate static lint notes/warnings — ~70 noise lines printed on every gate run and re-inlined twice in evidence.**
Evidence: each tester output (this run: twice) carries 6 "declares synthetic fixture ids" notes (one lists 41 ids), 17 stale date-pin warnings, 20 routing-named-schema warnings; the implementer's two recorded `prompt-lint` runs repeat the block twice more inside the 27,989-byte evidence capture the reviewer reads. fabro-0d77 already de-duplicated the *tiers*; the *note spam* is untouched, and the 17 date pins (>45 days) have cried wolf so long they're invisible.
Change: in `.fabro/scripts/prompt-lint.nu` and `.fabro/scripts/battery-runner.nu`, roll repeated notes into one line per class with a count (full detail behind `--verbose`); date pins collapse to a single roll-up.
Expected effect: roughly halves tester/evidence output, shrinks the implementer's bounce preamble and the reviewer's context, and restores date-pin signal.
Seed: **none covers note aggregation** — fabro-0d16 covers marker *name drift* only. *New-seed justification: dedup of tier execution (fabro-0d77) is closed; no open seed addresses per-note/warning roll-up in lint output.*

**4. Ship fabro-b769 (long-stage heartbeat) — 3-minute agent laps are silent.**
Evidence: notifications config fires only on `run.completed`/`run.failed` (run spec); implementer laps ran 185s and 143s, planner 150s, with zero interim signal; total run 10.4 min.
Change: workflow-level hook/notify on stage progress (node `implementer`/`planner`, or a `stage_progress` hook alongside the existing `stage-journal` hook in the loop `workflow.toml`).
Expected effect: the operator watching Slack sees liveness during inference-dominated laps instead of a 10-minute void.
Seed: **fabro-b769** (open, @loop).

**5. Ship fabro-d19e (nushell skill) carrying the probe patterns this run had to re-derive.**
Evidence: implementer@1's recorded checks show two failed probe iterations (nu parse errors: unbalanced delimiter, extra positional) before landing the env-var + string-concat quoting pattern; its journal flags exactly this as a recurring trap.
Change: `.fabro/skills/`-level nushell skill including the working nested-quoting probe template from this run's transcript.
Expected effect: saves 1–2 probe iterations (~30–60s + tokens) per implementer pass that shells out to nested `nu -c`.
Seed: **fabro-d19e** (open, @loop).

**6. Make prompt-lint's exit-2 "scope broken" a teaching error — the temp-root fixture contract is implicit.**
Evidence: implementer journal: the lint exits 2 unless the temp root replicates the conductor/develop/loop/revisor dir shapes; fixture authors discover this only by failing (the negative-path probe in this run had to reconstruct the whole layout under `mktemp -d`).
Change: `.fabro/scripts/prompt-lint.nu` scope check prints *which* expected dirs/files are missing; document the contract in the `.fabro/scripts/prompt-lint-fixtures.nu` header.
Expected effect: one-shot fixture authoring instead of exit-2 trial-and-error on every future prompt-lint check.
Seed: **none** — *New-seed justification: fabro-d19e covers nu semantics, not prompt-lint's own fixture-layout contract or its error message.*

**7. Under fabro-f18a, add a live-schema tool-call example for `fabro_run_set_title`/`fabro_run_search` in the planner prompt.**
Evidence: planner tool stats — `fabro_run_search: 2 calls, 1 error`, `fabro_run_set_title: 2 calls, 1 error` — each error is a wasted turn in the run's most expensive non-implementer stage.
Change: `.fabro/workflows/loop/prompts/planner.md` gains exact argument examples for both tools, then fabro-f18a's schema lint keeps them honest.
Expected effect: removes 1–2 retry turns per planning lap.
Seed: **fabro-f18a** (open, @loop) — adding the examples is its input; the lint itself is its deliverable.

---

Not recommended despite temptation: lowering `tester timeout="20m"` (actual runs: 14s — the headroom is for the dot-snapshot cold-compile tier, per the graph comment) and deduplicating the planner→exit edges (the four edges carry distinct conditions/kinds by design, from the graph source in the run projection).
