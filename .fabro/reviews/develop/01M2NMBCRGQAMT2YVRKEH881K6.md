# Improve review — run 01M2NMBCRGQAMT2YVRKEH881K6

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (3.7 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-16 17:39+0000 by revisor `fabro_ask`

---

All evidence gathered — run events (seq 1–213), worker log, and the tracker at `/workspace/fabro/.seeds/issues.jsonl` (cross-checked so every seed id below is a real, currently-open seed unless noted). Basis: this run claimed **fabro-ac84** (qualitygate fixture-battery tier), passed everything first-try: 3m30s wall, $0.236 total (planner $0.093 / 39%, implementer $0.107 / 46%, reviewer $0.035), 178s inference vs 16s tool time, PR #197 with auto-merge.

## Recommendations, by expected impact

**1. Land the engine-side seed-id projection for the in-flight check — fabro-9372 (with fabro-6b58).**
What happened: the planner's `fabro_runs_list` call (seq 33) returned 29 unbounded runs including *this run itself*; PR #183's generic goal forced the journal-grep fallback and **two extra LLM rounds** (seq 36–48, ~13s) to recover `fabro-b95d` — the exact heuristic 9372 documents. Sharper: open seed fabro-8dd8 (filed 17:09, from tracker) claims PR #195 already "wired battery execution into the gate," yet this run's tracker snapshot showed fabro-ac84 **open** and `dup-run-check` **clean** at 17:32 (seq 82) — either 8dd8 misattributes, or this run's base (4214d3c) predated #195's merge, the same ~1-minute race recorded in 9372/6b58's fresh evidence. I could not inspect PR #195's diff from here (no git/PR access) — that ambiguity is itself the finding.
Change: implement `current_seed_id` in the `fabro_runs_list` projection (engine), plus the 6b58 prompt line in `.fabro/workflows/develop/prompts/planner.md` step 4: `created_since ≈ 48h` + explicit self-run exclusion.
Effect: −2 LLM rounds per planner pass and the mis-skip/double-implement class becomes structural instead of heuristic.

**2. Top-N `sd ready` view — fabro-c3b4.**
What happened: `sd ready --assignee fabro --limit 200` poured **28,174 bytes / 200 seed rows** into the planner (seq 30, `stdout_truncated: true`) to pick one seed; the planner is already 39% of run cost at `reasoning_effort=low`.
Change: planner.md step 1–2 — top-N pick view (`--priority high` or first ~10), full listing only when top candidates are unclaimable.
Effect: several KB and ~4k tokens of context bloat removed per pass; compounds with rec 1.

**3. Validate verification-command syntax before a brief ships — *new seed needed*.**
What happened: the brief's cheapest-first criterion said `nu --ide-check scripts/qualitygate.nu`, which **errors** ("Provide a whole number for this option", seq 97 stderr); the implementer journaled it as a painpoint and self-corrected to `nu --ide-check 10 <file>`. Worse, the `;`-chained probe still exited 0, so the failure was stderr-only — one masked failure plus one recovery round. Planner.md step 7 mandates confirming *paths and headings* exist, never that *commands run*.
New-seed justification: fabro-3805 covers scan-scope runnability and fabro-4c81 covers path resolution; neither covers flag/argument syntax of brief verification commands.
Change: extend planner.md step 7 — dry-run or annotate every literal verification command in a brief before it ships.
Effect: eliminates one failed shell call + one LLM recovery round per brief carrying a transcribed-broken command.

**4. Document the `ml record` flag contract and print the mx-id — fabro-96bd + fabro-8d81.**
What happened: lesson capture failed once on `--name` (seq 126–128, exit 0 — error visible only in stdout text), then the implementer spent **two more calls** hunting the id (grep + python extraction, seq 138–146) because `ml record` doesn't print it. Three avoidable calls/rounds ≈ $0.03.
Change: implementer.md step 6 — per-type flag table (`pattern`→`--name`, `decision`→`--title/--rationale`); upstream, make `ml record` print the mx-id (8d81).
Effect: −3 tool calls and LLM rounds on every lesson-capturing pass.

**5. Stop `ml record` from mutating tracked config mid-run — fabro-b94d.**
What happened: the run diff includes `+ devloop: {}` in `.mulch/mulch.config.yaml` (auto-created domain), and the reviewer explicitly burned adjudication on it ("Mulch churn … adjudicated as explained loop machinery", seq 191) — per its journal observation.
Change: pre-create the `devloop` domain (toolchain image or prompt step), or classify the file as expected churn in `.fabro/workflows/develop/scripts/evidence.nu`.
Effect: removes recurring reviewer-scope noise from every lesson capture.

**6. Sweep satisfied sibling seeds on merge — fabro-ab38, with fabro-8dd8 as this run's live instance.**
What happened: this run's tester output (257 bytes) already prints `fixture battery green: .fabro/scripts/dup-run-check-fixtures.nu` — precisely the "success is silent" gap fabro-8dd8 demands fixing. 8dd8 is still open, so a future run can burn a cycle re-implementing an already-satisfied line.
Change: on merge of PR #197, run the ab38 sweep and close/rescope 8dd8 against the merged gate output (its basis run targeted the pre-per-battery-line state).
Effect: one fewer wasted future cycle; the planner's stale-basis check only audits the *claimed* seed, not siblings.

**7. Downgrade the by-design allow-key warn — fabro-8275.**
What happened: the worker log carries one WARN — `preamble_allow_keys entry absent from context node=implementer key=output.gate_known_bug_hits` — expected on any green first visit (gatebounce never ran).
Change: engine log policy, warn→info for declared-but-absent allow-keys.
Effect: warn-level logs regain signal (this run's only warning was noise).

**8. Log absent optional memory files at info — fabro-a701.**
What happened: six ERROR lines across the three agent sessions, all `File "/workspace/fabro/.codex/instructions.md" was not found` (from the worker log) — an optional file, identical spam per session init.
Change: engine session-init logging level.
Effect: error-level logs stop crying wolf three times per run.

**What needs no change (evidence the current prompting works):** the reviewer approved with a **single 42ms tool call** (seq 186–188) because the inline per-criterion PASS/FAIL report, the complete evidence capture, and the gate output in its preamble were sufficient — the verification-economy design did its job; the tester ran 4.2s (no crates touched) and closeout in 0.4s. The friction in this run is concentrated in planner context volume (recs 1–2) and small tool-contract traps (recs 3–5), not in the graph shape itself.
