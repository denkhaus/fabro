# Revision — run 01M65XEXP2AFDDP5TFNPMDAPEP

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M65XEXP2AFDDP5TFNPMDAPEP.md
- items filed: none — all eight findings either duplicate existing items (closed or filed by the run's own closeout window) or survive as overflow below; zero balance credit this pass
- balance: 0 non-exempt items filed / 0 — no credit this pass (no same-pass stale/superseded closes)
- basis: run 01M65XEXP2AFDDP5TFNPMDAPEP, workflow version absent, commit 79d8d9f06238dd06e45f55efb61c6ef09f7d4be4
- revised_at_commit: 79d8d9f06238dd06e45f55efb61c6ef09f7d4be4 (ADR-0015: engine drift signal for later judgement)

## Findings

**1. Resolve the planner-prompt contradiction that made the implementer re-run the whole tester battery** — duplicate_of

duplicate_of: denkhaus/work#962 — closed item demanded exactly this (loop planner.md Envelope note prescribes single-file parse probes; fix "landed" in 7b263ecf7). This run (event seq 58 + evidence-stage checks: full `just lint-nu` 65 scripts, `prompt-lint` 74 files, `validate-workflows` 22 graphs, re-run by the tester 14s later) proves the landed fix does not hold — the envelope note still names tier names (lint-nu, prompt-lint, validate) in brief criteria. REGRESSION signal, journal-recorded for the human gate; not re-filed because the concrete change (rewrite that envelope note) is what #962 already names.

**2. Make silently-skipped gate tiers visible (GATE DEGRADED block in loop-gate.nu)** — overflow
- overflow: Aggregate skipped/suspended gate tiers into a trailing `GATE DEGRADED` block in loop-gate.nu — in `.fabro/workflows/loop/scripts/loop-gate.nu`, aggregate every skipped/suspended tier (this run: `seed-spec-paths: work claims failed — skipping (fail-open)` and `prompt-lint: tracker census failed — resolvable-id tier suspended`) into a trailing `GATE DEGRADED: <tier> (<reason>)` block, and mirror the census error path in `.fabro/scripts/prompt-lint.nu`; effect: "LOOP GATE GREEN" regains meaning — a skipped empty-edit detector can no longer hide behind a green banner the reviewer trusts (this run went green with two deterministic guarantees absent). Complements open denkhaus/work#1034 (fixes the seed-spec tier's transport cause; this is degradation visibility for ANY skipped tier). Dedupe this pass: `loop-gate degraded skipped tiers` → 0 rows; `gate` overview scanned, no item names skipped-tier aggregation.

**3. Enforce "final message = routing JSON only" in the implementer prompt** — overflow
- overflow: Implementer final message must be the routing JSON object alone — one line in `.fabro/workflows/loop/prompts/implementer.md` (mirror develop): the routing JSON object is the ENTIRE final message, no prose before/after, no bracket-wrapped arrays; effect: eliminates the repair turn burned this run (implementer output: "the routing JSON object is malformed: key must be a string at line 6 column 531; repair turn 1" — a wrapped-in-prose JSON plus a broken second copy) on the stage that is 76% of run cost ($0.4735 of $0.6212). Distinct from closed denkhaus/work#723 (engine-side early-fail for planner prose; this is the implementer prompt ergonomics rule). Dedupe this pass: `implementer routing JSON final message` → no item covering the prompt rule.

**4. Fix `fabro_run_set_title` ergonomics (report limit and actual length)** — duplicate_of

duplicate_of: denkhaus/work#1075 — the run's own closeout window already filed this exact demand citing this exact run (planner tool stats: 4 calls, 3 errors; three retries). No filing claim remains.

**5. Pin the petri-cli probe build line where the lane reads facts** — duplicate_of

duplicate_of: denkhaus/work#1076 — same window, same run's evidence (no `petri` package; `-p petri-cli` cost ~90s across two wrong selectors); already filed with the `mise run dev` alternative arm.

**6. Aggregate prompt-lint's per-file note/warn spam** — overflow-dup

- overflow-dup: Batch prompt-lint synthetic-id fixture notes to one summary line (open in 01M5CPBDF5PNC8EGDK805SVW10.md) — this run extends the theme with a warn-aggregation arm: collapse the ~60 per-file resolvable-id notes AND summarize the ~26 date-pin + 10 routing-named warns (count + top offenders) in `.fabro/scripts/prompt-lint.nu`; measured effect this run: 14,337 bytes tester output, the same block verbatim again in the evidence capture (30,291 bytes), reviewer ingesting 30,443 input tokens. Adjacent open items are different mechanisms: denkhaus/work#332 (filters at loop-gate.nu by diff), denkhaus/work#334 (refreshes the pins themselves).

**7. Stop pointing later stages at `/tmp` probe artifacts** — overflow
- overflow: Probe proof stays on stdout via the check-transcript wrapper — one line in `.fabro/workflows/loop/prompts/implementer.md` (mirror develop): never redirect probe output to `/tmp/...` for anything a later stage must read; recorded checks stay self-contained on stdout; effect: the reviewer stops losing a tool call per redirected probe (this run: reviewer `read_file` 5 calls, 1 error — `/tmp/graph-after.txt` unreadable post-sandbox, per the reviewer's own observation). Dedupe this pass: `probe artifacts tmp` → no item covers probe-artifact placement in prompts.

**8. Filter the run's own journal out of evidence's dirty-worktree line** — overflow
- overflow: Exclude the run's own journal from the evidence dirty-worktree count — in `.fabro/workflows/develop/scripts/evidence.nu`, exclude `.fabro/journal/<this-run-id>.jsonl` from the dirty-worktree line (or label it "expected run artifact"); effect: the integrity line stops crying wolf every run (this run: `worktree=dirty: 1 entries — M .fabro/journal/01M65XEXP2AFDDP5TFNPMDAPEP.jsonl` — the run's own artifact flagged as residue), so genuine leftover hunks stand out to the planner's residue audit. Dedupe this pass: `evidence dirty worktree journal` → 0 rows; not present in the open overflow ledger.
