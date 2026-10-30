# Revision — run 01M673H9C4QXK4R1S7FT5WC8JA

- status reviewed: succeeded (34m11s wall, $1.36 total, all 8 stages first-pass green)
- review: .fabro/reviews/develop/01M673H9C4QXK4R1S7FT5WC8JA.md
- items filed: none — healthy run; 0 credit this pass (no same-pass stale/superseded closes), all surviving findings journaled as overflow per ADR-0022
- balance: 0 / 0 — no credit this pass
- basis: run 01M673H9C4QXK4R1S7FT5WC8JA, workflow version absent, commit 2287ebb8ef9874ed118b7ac63e6c691bd0f5bac5
- revised_at_commit: 2287ebb8ef9874ed118b7ac63e6c691bd0f5bac5 (ADR-0015: engine drift signal for later judgement)

## Findings

Closeout re-check (`fabro_work_list` query `01M673H9C4QXK4R1S7FT5WC8JA`): only the
reviewed item itself (`denkhaus/work#1065`, closed) — the run's closeout filed nothing
that covers any finding below.

NOTE for the next filing pass: findings 1, 4, and 5 all target
`.fabro/workflows/develop/prompts/implementer.md` — consolidate them into ONE multi-arm
item (structured arms schema) when re-filing.

**1. Exemption channel: mandate the `exemption: ` marker and sweep the `deferred-action:` synonym** — overflow
- overflow: Exemption channel misses deferred arms — mandate the exact `exemption: ` marker for run-unlandable arms in `.fabro/workflows/develop/prompts/implementer.md` (ban ad-hoc synonyms like `deferred-action:`) and widen the sweep in `.fabro/workflows/develop/scripts/closeout.nu` to also match `deferred-action:` as a legacy synonym; effect: no deferred arm dies with its closed item — in this run the TS client regen (acceptance criterion 1 of `denkhaus/work#1065`) was journaled as `deferred-action:` (no java/openapi-generator in sandbox), the word "exemption" appears exactly once in the event stream (planner prompt text), and closeout filed zero items, leaving the TS client out of sync with the new `laneWorkItem` endpoint. Cross-ref: closed `denkhaus/work#684` landed the implementation-summary sweep — this is the vocabulary-mismatch remainder, not a duplicate of its landed mechanism.

**2. Implementer final verification scope: new tests by name filter only** — overflow
- overflow: Implementer re-ran the tester's gate — in `.fabro/workflows/develop/prompts/implementer.md`, final verification = fmt/clippy on touched crates plus NEW tests by name filter only, with an explicit line that the full touched-crate suite is the tester node's exclusive job; effect: removes a duplicated full-crate nextest slice from the longest stage (implementer = 26m01s wall, 76% of run wall, $1.15 = 84.5% of cost, 86 tool calls; tester then re-ran the identical five-crate slice, 5m18s) — roughly 3–5 min wall per run, scaling with crate count. Prompt-adherence drift: the brief already prescribed the narrow scope; no tracker item covers implementer verification scope (tracker queries on exemption/evidence/expertise ran clean this pass; re-run dedupe on `verify`/`scope` before filing).

**3. Consult the expertise store before writing wire assertions** — overflow
- overflow: Known expertise not consulted at test-writing time — add a mandated pre-test step in `.fabro/workflows/develop/prompts/implementer.md`: shell-grep `.mulch/expertise/dev-loop.jsonl` for the touched file paths (loop assets are hidden from file tools, not from shell) and apply matching entries before writing wire assertions; effect: skips constraint-probe loops — this run's implementer binary-searched httpmock body constraints one cargo-test probe at a time (~5s each) to re-derive mx-4f2341 (`body_excludes`, Progenitor serializes `None` as an absent key), a measurable chunk of the implementer's 16m53s tool time. Distinct from open `denkhaus/work#120` (nushell skill, different knowledge source).

**4. Run-title length rule in the develop planner prompt** — overflow
- overflow: Run-title 100-char limit burned two failed tool calls — add one hard line to the RUN TITLE paragraph of `.fabro/workflows/develop/prompts/planner.md`: "≤100 characters — count before calling; prefer ≤80" (deterministic alternative: derive a fallback title in `planner-preflight.nu`); effect: 1 tool call instead of 3 and ~2 LLM turns saved per run (`fabro_run_set_title` failed twice with "run title must be at most 100 characters", planner events seq 167–183, inside a reasoning_effort=low stage) — a guaranteed-recurring tax since titles are mandatory per-run. Cross-ref: open `denkhaus/work#323` demands raising the engine cap (different mechanism, complementary — cross-reference it, not supersession).

## Linked (deduped against the open overflow ledger)

- overflow-dup: Re-pin the staged FACTS merge-target ref (open in 01M5K62554W7YCVZ7W4AT8YYWK.md) — finding "merge-target branch `origin/denkhaus` never fetched; planner grep and `planner-preflight.nu` dup-run-check run against the wrong history (`git branch -r` shows only `origin/main`; `git log origin/denkhaus` = fatal: ambiguous argument, planner events seq 71–81)" is the same theme: make `origin/denkhaus` fetchable/repinned so already-landed detection works. Not re-journaled as a full bullet.

## Duplicates (dropped, never overflow)

- duplicate_of: denkhaus/work#94 — finding "evidence capture demoted the spec-first yaml to truncated churn" (promote spec-named doc paths from churn to seed work in `evidence.nu`, order docs before Rust diffs / lift the 30-line churn cap) names the same concrete change #94 demanded; #94 closed 2026-10-29 yet the run shows `fabro-api.yaml` (+75 lines) still classified as loop churn and truncated mid-hunk ("45 further changed lines not shown"), forcing the reviewer into 4 tool calls (2 grep, 2 read_file at lines 6798+/18802+). The recurrence against a closed item is itself a signal: next pass should verify whether #94's fix actually landed or regressed before re-filing.
- duplicate_of: denkhaus/work#887 — finding "judgment-shadow hook failed at every stage_complete" (non_blocking_hook_failed warn at node start seq 27, hook-failure payloads at every AfterVisit since, each ~22ms, never succeeded once) is the same defect #887 recorded; #887 is closed. Same recurrence-against-closed-item caveat as above.

Basis: run 01M673H9C4QXK4R1S7FT5WC8JA, workflow version absent, commit 2287ebb8ef9874ed118b7ac63e6c691bd0f5bac5
