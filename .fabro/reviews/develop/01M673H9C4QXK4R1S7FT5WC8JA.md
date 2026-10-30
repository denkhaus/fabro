# Improve review — run 01M673H9C4QXK4R1S7FT5WC8JA

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (34m11s wall, $1.36 total, revisor pass — reason and cost in run detail)
- generated: 2026-10-30 21:33+0100 by revisor `fabro_ask`

---

# Recommendations for the Develop workflow — grounded in run 01M673H9C4QXK4R1S7FT5WC8JA

**Run baseline (from run events / conclusion):** 34m11s wall, $1.36 total, all 8 stages first-pass green, zero retries. Implementer = 26m01s wall (76%) and $1.15 (84.5%) across 86 tool calls (62 shell); tester 5m18s; planner 1m41s; reviewer 31.5s. The graph topology itself performed flawlessly — every recommendation below is about scripts, prompts, or environment wiring, not routing.

**Tracker dedupe (searches I ran):** `fabro_seeds_search` with queries *"evidence capture diff ordering truncation"*, *"merge-target fetch origin/denkhaus"*, *"run title length set_title"*, *"judgment-shadow hook"*, *"verify implementer test scope gate duplicate"*, then single terms *"evidence"*, *"title"*, *"hook"*, *"fetch"*, *"preamble"*, *"qualitygate"*, then unfiltered lists (all statuses) — **all returned 0 rows**. The live work tracker is server-side behind the run's `fabro_work_*` tools, which I don't hold, so I could not confirm or rule out existing items; every recommendation therefore carries a one-line new-item justification. Re-dedupe against `fabro_work_list` before filing.

---

## 1. The EXEMPTION channel missed a real deferred arm — the TS client regen is now lost work
**What happened (from run events):** the implementer's journal recorded `"deferred-action: regenerate the TypeScript client … the sandbox has no java/openapi-generator"` — part of the item's own acceptance criterion (1). But the planner prompt's contract requires the marker `exemption: ` for closeout's sweep to re-file the arm; the word "exemption" appears exactly **once** in the entire event stream — inside the planner's prompt text. Closeout's output shows only `closed denkhaus/work#1065`, **zero filings**. The generated TS client is now out of sync with the new `laneWorkItem` endpoint and nothing in the tracker remembers it.
**Change:** `.fabro/workflows/develop/prompts/implementer.md` — mandate the exact `exemption: ` marker for any run-unlandable arm (ban ad-hoc synonyms like `deferred-action:`); and `.fabro/workflows/develop/scripts/closeout.nu` — widen the sweep to also match `deferred-action:` as a legacy synonym.
**Expected effect:** no deferred arm ever dies with its closed item; in this run it would have auto-filed the TS regen as an open ops item.
**New-item justification:** the marker-vocabulary mismatch broke the exemption channel in this run and no item in my (empty) tracker view covers the closeout sweep's matching rules.

## 2. Evidence capture demoted the spec-first yaml to truncated churn — reviewer had to detour
**What happened (from evidence@1 output + reviewer@1 journal):** `fabro-api.yaml` (+75 lines — the item's criterion-(1) deliverable) was classified as *loop churn* and its diff was cut mid-hunk ("45 further changed lines not shown — churn diffs cap at the first 30 changed lines") inside the 31.9KB capture. The reviewer then burned 4 tool calls (2 grep, 2 read_file at lines 6798+/18802+) to see the response codes and `WorkItemLaneRequest` schema, and filed this exact painpoint: *"put the OpenAPI spec diff before the Rust diffs … or cap per-file diff hunks so the last file isn't the one always truncated."*
**Change:** `.fabro/workflows/develop/scripts/evidence.nu` — promote spec-named doc paths (paths the item body names, e.g. `docs/public/api-reference/**`) from the churn section into seed work, and order docs diffs before Rust diffs (or lift the 30-line churn cap for spec-named files).
**Expected effect:** API-item reviews verify the contract from the capture directly — no read detours, and no risk of a "Verification blocked" bounce (~3.5 min/cycle per the graph's own history) when truncation hides the wrong hunk.
**New-item justification:** the reviewer's painpoint was filed in the journal but closeout filed nothing, and my tracker view shows no item for evidence capture ordering.

## 3. Merge-target branch is never fetched — already-landed detection greps the wrong history
**What happened (from planner events seq 71–81 + planner@1 journal):** `git log origin/denkhaus` failed (`fatal: ambiguous argument`); `git branch -r` shows only `origin/main`. PROJECT_FACTS *mandates* grepping the merge-target `origin/denkhaus` for branch-sensitive checks; the planner silently fell back to local history and journaled the gap. A fix that landed on the target branch would be invisible → a duplicate ~34min/$1.36 run, the exact failure the "Already landed" arm exists to prevent.
**Change:** `.fabro/workflows/develop/workflow.toml` — add a `[run.prepare]` step (or clone refspec) fetching `origin/denkhaus`, so both the planner's grep and `planner-preflight.nu`'s dup-run-check run against the real merge-target.
**Expected effect:** the dedupe arm works as designed; eliminates the duplicate-implementation run class on this branch layout.
**New-item justification:** the fetch gap is a run-environment wiring defect named in this run's journal; nothing in my tracker view covers checkout refspecs/prepare steps.

## 4. The implementer re-ran the tester's gate — duplicated the most expensive slice
**What happened (from implementer@1 summary + tester@1 log):** the brief prescribed *"run new tests by name filter … gate green via the deterministic tester step"*, but the implementer ran `just verify implementer` = fmt + clippy + **nextest over all five touched crates**, and the tester then re-ran the identical five-crate nextest slice (5m18s, warm only because the implementer had just built it). The implementer owns 84.5% of run cost; its verification step widened scope by hand against the prompt's own test-scoping rule.
**Change:** `.fabro/workflows/develop/prompts/implementer.md` — final verification = fmt/clippy on touched crates + **new tests by name filter only**; state explicitly that the full touched-crate suite is the tester node's exclusive job.
**Expected effect:** removes a duplicated full-crate test slice from the longest stage — on this run roughly 3–5 min of wall and its share of $1.15; scales with crate count on bigger items.
**New-item justification:** this is prompt-adherence drift measured in this run (brief prescribed narrow scope, implementer ran wide); no tracker item in my view covers implementer verification scope.

## 5. Known expertise wasn't consulted before test-writing — the httpmock probe loop re-derived mx-4f2341 live
**What happened (from implementer@1 lesson mx-d4cced):** the implementer binary-searched httpmock body constraints one at a time (~5s per cargo test probe) to discover that Progenitor serializes `None` as an **absent** key — a trap already recorded as mx-4f2341 (`body_excludes`) in `.mulch/expertise/dev-loop.jsonl`, and re-derived here because nothing surfaces it at test-writing time. The lesson also warns a pre-edit test run can execute a stale binary.
**Change:** `.fabro/workflows/develop/prompts/implementer.md` — add a mandated pre-test step: shell-grep `.mulch/expertise/dev-loop.jsonl` for the touched file paths (loop assets are hidden from file tools, not from shell) and apply matching entries before writing wire assertions.
**Expected effect:** skips constraint-probe loops and stale-FAIL misreads; on this run that debugging sequence was a measurable chunk of the implementer's 16m53s tool time.
**New-item justification:** the expertise store exists but has no consult trigger in the implementer prompt; no item in my view covers expertise surfacing.

## 6. Run-title 100-char limit burned two failed tool calls — recurs every run
**What happened (from planner events seq 167–183):** `fabro_run_set_title` failed twice with *"run title must be at most 100 characters"* before the third attempt succeeded — 2 wasted tool calls plus 2 extra LLM round-trips inside a `reasoning_effort=low` stage that demonstrably doesn't count characters. Titles are mandatory per-run, so this tax repeats on every run.
**Change:** `.fabro/workflows/develop/prompts/planner.md` — the RUN TITLE paragraph gains one hard line: "≤100 characters — count before calling; prefer ≤80". (Deterministic alternative worth considering: derive a fallback title in `planner-preflight.nu` from the granted-item packet.)
**Expected effect:** 1 tool call instead of 3 and ~2 LLM turns saved per run; removes a guaranteed-recurring failure.
**New-item justification:** tool-description text alone failed to prevent the error in this run; no tracker item in my view covers the title-length contract.

## 7. `judgment-shadow` hook failed at every stage_complete — dead telemetry + warn noise
**What happened (from run events):** `run.notice` warn `non_blocking_hook_failed` at node `start` (seq 27), and hook-failure payloads at every AfterVisit since — env_guard (seq 41), planner (196), preflight (210), implementer (897), tester (947/1029), evidence (1738): "hook exited with code 1", each ~22ms, all ignored because non-blocking. The hook never succeeded once this run, so whatever judgment telemetry it exists to collect was silently not collected.
**Change:** `.fabro/scripts/judgment-shadow.nu` — make its not-applicable path exit 0 (or fix the code-1 cause), or unregister the hook in the workflow settings until fixed.
**Expected effect:** removes one warn notice per stage from every run's event stream and restores the shadow-judgment measurement this run completely lacked.
**New-item justification:** a persistently failing non-blocking hook is invisible in run status; nothing in my tracker view covers judgment-shadow health.

---

**What I could not inspect:** the live `denkhaus/work` tracker behind `fabro_work_*` (my seeds-search surface returned 0 rows for every query), so items #1–#7 may overlap existing server-side work items — dedupe with `fabro_work_list {"query": ...}` per theme before filing. The graph itself needs no topology change on this run's evidence: single-pass, no bounces, all guards untriggered.
