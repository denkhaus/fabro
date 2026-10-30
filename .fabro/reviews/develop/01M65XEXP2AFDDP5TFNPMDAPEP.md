# Improve review — run 01M65XEXP2AFDDP5TFNPMDAPEP

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (10m18s wall / $0.62, revisor pass — reason and cost in run detail)
- generated: 2026-10-30 21:19+0100 by revisor `fabro_ask`

---

## What this run actually did (grounding)

Run `01M65XEXP2AFDDP5TFNPMDAPEP` (loop lane) claimed and closed `denkhaus/work#1060` (stage-journal hook → project layer), PR #501. Succeeded in **10m18s wall / $0.62**, zero retries at the graph level. The **implementer stage alone was 449.5s wall (73%) and $0.4735 of $0.6212 (76%)** across 50 tool calls (from run conclusion/stage timings). All findings below are from run events and stage outputs.

**Tracker check (required):** I searched the tracker before every recommendation — queries `set_title title limit 100`, `petri print-graph build cli`, `prompt-lint`, `fail-open`, `seed-spec-paths`, `routing json output contract repair`, `implementer lint battery duplicate`, `noise notes warnings battery output`, `title`, `petri`, `lint`, plus an **unfiltered list** — all returned **0 rows** (the tracker view I can read is empty; the only known key in run evidence, `denkhaus/work#1060`, is this run's own closed item and covers none of these). So each recommendation carries a new-item justification.

## Recommendations, by expected impact

**1. Resolve the planner-prompt contradiction that made the implementer re-run the whole tester battery.**
What happened: the planner prompt (event seq 58) says *"whole-repo lint and battery tiers belong EXCLUSIVELY to the tester node; the implementer never runs them"* — yet its own envelope note tells the planner to write criteria *"name parse-level checks (lint-nu, prompt-lint, validate)"*. Result: the implementer's recorded checks (evidence stage output) include full `just lint-nu` (65 scripts), full `nu .fabro/scripts/prompt-lint.nu` (74 files), and `validate-workflows` (22 graphs) — all re-run by the tester **14 seconds later**.
Change: in `.fabro/workflows/loop/prompts/planner.md`, rewrite the "Envelope note for the brief" to prescribe single-file probes (`nu -c "source <script>"`, exact-string greps) and ban tier names from brief criteria.
Expected effect: removes 3 whole-repo runs from the stage that is 76% of run cost and 73% of wall time; gate coverage unchanged.
*New item — justification: no tracker row exists (12 searches above returned 0); this is loop-lane prompt work, not covered by the closed work#1060.*

**2. Make silently-skipped gate tiers visible — this run went GREEN with its empty-edit detector off.**
What happened: tester output contains `seed-spec-paths: work claims failed — skipping (fail-open)` and `prompt-lint: tracker census failed — resolvable-id tier suspended this run: census unavailable: External command failed` — two deterministic guarantees absent, final line still `LOOP GATE GREEN`, and the reviewer saw only "Approved".
Change: in `.fabro/workflows/loop/scripts/loop-gate.nu`, aggregate every skipped/suspended tier into a trailing `GATE DEGRADED: seed-spec-paths (work claims failed: …)` block (strict mode optional); mirror the census error path in `.fabro/scripts/prompt-lint.nu`.
Expected effect: "green" regains meaning — a skipped seed-spec-paths tier (the wrong-file/empty-edit detector per PROJECT_FACTS) can no longer hide behind a green banner the reviewer trusts.
*New item — justification: tracker empty; degradation visibility is new ground (fabro-905d-class blind spot, but no open row).*

**3. Enforce "final message = routing JSON only" in the implementer prompt.**
What happened: implementer output shows `the response does not meet the output contract (the routing JSON object is malformed: key must be a string at line 6 column 531); repair turn 1` — it wrapped the JSON in prose and emitted a broken second copy (`[…], ["observations": …`), burning a repair turn on the most expensive stage.
Change: one line in `.fabro/workflows/loop/prompts/implementer.md`: the routing JSON object is the *entire* final message — no prose before/after, no bracket-wrapped arrays (planner and reviewer already comply unprompted).
Expected effect: eliminates the repair turn (extra LLM round-trip ≈ tens of seconds and tokens on the $0.47 stage) whenever the implementer summarizes.
*New item — justification: tracker empty; contract-repair ergonomics for this node are uncovered.*

**4. Fix `fabro_run_set_title` ergonomics — 3 of 4 calls failed this run.**
What happened: planner tool stats (planner stage, `agent.tools`): `fabro_run_set_title: 4 calls, 3 errors`; the planner's own journal painpoint: *"rejects titles over 100 chars with no length hint … three retries needed"* — each retry is a full LLM round-trip, every run.
Change: engine-side (product lane, `lib/`): either server-side truncate or return `limit 100, got N` in the rejection; the tool description already states the limit and the model still re-offends.
Expected effect: ~3 wasted planner turns per loop run removed (~15–30s + tokens each; planner costs $0.076/run).
*New item — justification: tracker empty and this is a product-surface change, out of loop-lane scope; needs its own product-lane item.*

**5. Pin the petri-cli probe build line where the lane reads facts.**
What happened: implementer journal painpoint: no `petri` binary exists, package is `petri-cli`, and *"building it took two wrong `-p` selectors before `-p petri-cli` worked (~90s total)"*; its observation also notes `fabro validate --json` doesn't expose `Graph.params["attractor.hooks"]`, forcing a Rust build for a one-line proof.
Change: add to `.fabro/workflows/loop/prompts/project-facts.md` (the lane's single facts block, ADR-0013): `graph/hooks probe: cd vendor/petri && CARGO_TARGET_DIR=$(mktemp -d) cargo build --locked -p petri-cli; petri check --print-graph <graph>`; longer term, expose hooks in `validate --json` (product item).
Expected effect: any future hooks/graph-layer item skips the 90s two-failed-builds rediscovery on the stage that dominates run time.
*New item — justification: tracker empty; the planner's painpoint was filed to journal only (loop lane has no lesson_capture → revisor never saw a seed).*

**6. Aggregate prompt-lint's per-file note/warn spam.**
What happened: the battery prints ~60 `resolvable-id check skipped for N ids this run` notes, ~26 date-pin warns and 10 routing-named warns — 14,337 bytes of tester output, then the **same block verbatim again** inside the evidence capture (30,291 bytes), which the reviewer then ingests (30,443 input tokens).
Change: in `.fabro/scripts/prompt-lint.nu`, collapse per-file notes to one aggregate line per tier and summarize warns (count + top offenders); keep synthetic-fixture exemptions explicit.
Expected effect: ~10–20KB less duplicated output per run, cheaper reviewer context, and the 26 real "still load-bearing?" date-pin warnings stop being buried.
*New item — justification: tracker empty; noise-reduction of this lint is uncovered.*

**7. Stop pointing later stages at `/tmp` probe artifacts.**
What happened: reviewer `read_file`: 5 calls, 1 error — `/tmp/graph-after.txt` from the implementer's redirected probe is unreadable post-sandbox (reviewer's own observation: *"unreadable post-sandbox"*).
Change: one line in `.fabro/workflows/loop/prompts/implementer.md`: probe proof stays on stdout via the check-transcript wrapper; never `> /tmp/...` for anything a later stage must read.
Expected effect: reviewer stops losing a tool call per redirected probe; recorded checks stay self-contained.
*New item — justification: tracker empty; no row covers probe-artifact placement.*

**8. Filter the run's own journal out of evidence's dirty-worktree line.**
What happened: evidence integrity reports `worktree=dirty: 1 entries — M .fabro/journal/01M65XEXP2AFDDP5TFNPMDAPEP.jsonl` — the run's own artifact, flagged every run as if it were residue.
Change: in `.fabro/workflows/develop/scripts/evidence.nu`, exclude `.fabro/journal/<this-run-id>.jsonl` from the dirty count (or label it "expected run artifact").
Expected effect: the integrity line stops crying wolf; genuine leftover hunks (the thing the planner's residue audit looks for) stand out.
*New item — justification: tracker empty; evidence-noise hygiene is uncovered.*

**Not worth changing (evidence says leave alone):** `env_guard` (42ms, zero cost — the fabro-3ab2 design works), the tester's 20m timeout (actual 13.2s), closeout's 7.4s claim-bound close, and the graph's deadlock/soft-exit edges — none fired, and the conditional bounce caps (work#1029/#1057) were not exercised.
