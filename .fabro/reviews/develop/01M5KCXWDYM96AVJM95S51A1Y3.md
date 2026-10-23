# Improve review — run 01M5KCXWDYM96AVJM95S51A1Y3

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded(completed) (11m32s, revisor pass — reason and cost in run detail)
- generated: 2026-10-23 04:51+0000 by revisor `fabro_ask`

---

## Evidence base (from run events + stage transcripts for run 01M5KCXWDYM96AVJM95S51A1Y3)

Clean first-pass run: 11m32s wall, **$0.390 total**, 0 retries, seed fabro-1923 claimed → implemented (+2/−2 in `revisor/prompts/analyze.md`) → gate green → approved → closed → PR #477. Cost/time split: **planner $0.203 / 307s (52% of cost)**, implementer $0.134 / 285s, reviewer $0.053 / 44s, all deterministic nodes < 17s combined. The waste is concentrated in the planner and in duplicated lint output — that drives the ordering below.

Dedupe searches run against the tracker (all named so you can re-verify): `top-N ready firehose planner`, `English implementer prompt`, `tester output twice dedupe evidence`, `diff-scope prompt-lint loop gate`, `in-flight planner check bound`, `title length set_title`, `precondition brief planner seed`, `implementer single-file probes battery double`, `fabro-30a5`, `architect analyze vocabulary item-key`, `architect`, `run_search status enum valid`.

---

**1. Stop feeding the planner the 65-row `seeds ready` firehose — the preflight table already exists. → item `fabro-c3b4` (open)**
- What happened: the preflight node handed the planner a 5-candidate verdict table (`fabro-1923…fabro-c03a`, clean/envelope_risk, inline as `output.preflight`), then planner.md step 1 made it re-run `seeds ready --assignee loop --limit 200` — 65 rows, ~9.4KB, 37 irrelevant seeds adjudicated away — inside the run's most expensive stage (15 messages, 18 tool calls, $0.203).
- Change: `.fabro/workflows/loop/prompts/planner.md` step 1 — treat `output.preflight` as the primary candidate list; `seeds ready` drops to a top-N confirmation (`--limit 10`) only for the chosen candidate.
- Effect: several KB and 2–3 LLM turns cut from the stage that is 52% of run cost; expected ~15–25% total-run cost reduction.

**2. Pin the planner's tool-call contracts (run-status enum, 100-char title cap). → new item**
- What happened (planner agent tool stats, from run events): `fabro_run_search` 2 calls / **1 error** — `"unknown run status 'in_progress'"` (planner guessed `in_progress, queued, pending, running`; the engine's statuses are `submitted/runnable/starting/running`); `fabro_run_set_title` 3 calls / **2 errors** — titles over 100 chars rejected, accepted only on the third attempt. Three wasted calls, three recovery round-trips in the costliest stage.
- Change: `.fabro/workflows/loop/prompts/planner.md` (step 5 + the RUN TITLE section): name the valid non-terminal `status` values for `fabro_run_search` and state the 100-char title cap with "count before calling."
- Effect: eliminates 3 dead tool calls + 3 LLM turns per run; also removes the risk the in-flight exclusion silently degrades after an error.
- New-item justification: `title length set_title` and `run_search status enum valid` matched 0 open items; `fabro-6b58` covers only the created_since window/self-exclusion, not the tool argument contracts.

**3. Fix the brief-level regression that re-introduced the double battery run. → new item**
- What happened: planner.md's "Implementer pre-gate scoping" says briefs prescribe single-file probes only and cites run 01M550CAX's 21.6s double-battery cost — yet THIS run's brief (planner output, `current_seed_brief`) said "verification is lint-nu/prompt-lint/validate parse-level," and the implementer's recorded checks ran `prompt-lint` (77 files), `just lint-nu` (65 scripts), and `check-run-scope` — all three re-run by the tester node ~3 minutes later (loop-gate output). The landed rule regressed at the brief-vocabulary level.
- Change: `.fabro/workflows/loop/prompts/planner.md` "Envelope note for the brief" — replace "parse-level checks (lint-nu, prompt-lint, validate)" with "single-file probes (`nu -c 'source <script>'`, exact-string grep); whole-repo tiers belong to the tester alone."
- Effect: restores the 01M550CAX fix — saves ~10–15s tool time plus an implementer turn spent babysitting whole-repo lints, every run.
- New-item justification: `implementer single-file probes battery double` matched 0; the original fix lives only as prompt prose, and this run proves the prose is insufficient (the brief template contradicts it).

**4. Stop the reviewer reading the same lint output twice. → items `fabro-17fb` (open) and `fabro-a827` (open)**
- What happened: for a +2/−2 one-file diff, the reviewer preamble inlined the tester output (8,704 bytes: validate + battery incl. full prompt-lint with 37 warnings) AND the evidence output (12,580 bytes) whose "recorded checks" section re-prints the same prompt-lint/lint-nu transcripts — ~21KB duplicated into a 25.3K-token reviewer input.
- Change: `fabro-17fb` → `.fabro/workflows/develop/scripts/evidence.nu` (shared, `--lane loop`): when the tester stage already ran the battery, emit per-check exit-code lines instead of full transcripts; `fabro-a827` → `.fabro/workflows/loop/scripts/loop-gate.nu`: diff-scope the prompt-lint tier so only warnings on changed files print.
- Effect: ~15–20KB less reviewer context per review → cheaper, faster, less-distracted reviews; also shrinks every future gate-red bounce the implementer reads.

**5. Require English responses from the implementer. → item `fabro-eaa0` (open)**
- What happened: implementer@1's entire response is in German ("Alle Checks grün, Diff minimal…") with `fidelity=summary:high`, so that text flowed into the run record, checkpoint, and downstream review context; the reviewer worked in English.
- Change: `.fabro/workflows/loop/prompts/implementer.md` — one line: all prose and `implementation_summary` in English.
- Effect: consistent run/PR record and zero re-interpretation overhead in review; near-zero cost to implement.

**6. Briefs must state precondition status (met / waived) for named dependency seeds. → new item**
- What happened: the implementer journaled this exact painpoint (from run events): fabro-1923's action was conditional on fabro-4653 (still open at claim time) and "no planner brief reached the implementer context (only claim_check's ok line) — the precondition judgment fell to the implementer." The planner had in fact adjudicated the waiver (its journal observation) but the brief never carried it forward.
- Change: `.fabro/workflows/loop/prompts/planner.md` step 7 — when the seed body names other seed ids, add one brief bullet per named precondition: `met` / `waived because <one line>`.
- Effect: precondition risk surfaces at brief time and is reviewable, instead of being silently re-decided mid-implementation.
- New-item justification: `precondition brief planner seed` matched 0; `fabro-30a5` (filed by this run's closeout) tracks the fabro-4653 dependency itself, not the brief-disclosure contract.

**7. File the architect-lane vocabulary drift both roles flagged. → new item (low priority)**
- What happened: implementer AND reviewer independently journaled `.fabro/workflows/architect/prompts/analyze.md:67` still requiring "a known seed id … new-seed justification" — out of fabro-1923's scope, deliberately untouched twice.
- Change: file a seed for the architect lane's analyze-prompt migration to the ADR-0032 item-key vocabulary (touch `architect/prompts/analyze.md:13/15/67` as one unit when that lane migrates to `fabro_work_*`).
- Effect: closes the last lane speaking the retired seeds-CLI vocabulary to analysts, so Ask-Fabro deduped recommendations stay uniform across lanes.
- New-item justification: `architect analyze vocabulary item-key` and the broad `architect` search matched no open item about the architect workflow's prompt vocabulary (only `architecture`-labelled unrelated seeds); `fabro-4653`/`fabro-30a5` cover the loop lane only.

Not inspectable from here: the PR #477 review thread and the `judgment-shadow` hook's captured judgments (hook ran, 22–24ms per stage, but its output isn't in the run stream) — neither changed the recommendations above.
