# Improve review — run 01M2KC8BDP4AJZ5NR0CY2KF5JZ

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (1.9 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-15 20:38+0000 by revisor `fabro_ask`

---

All evidence below is from this run's events (`fabro_run_events`), worker log (`fabro_run_logs`), and the run summary. Baseline: 96.5 s wall, $0.164 LLM cost, zero retries, all stages first-pass green — planner $0.0705/34.6 s (43% of cost, 29,476 input tokens), implementer $0.0572/27.7 s, reviewer $0.036/15.6 s, tester 3.6 s. Seed existence was checked against the live `sd ready` listing captured in this run (seq 31).

## Recommendations, by expected impact

**1. Stop feeding the planner a 200-seed listing — top-N `sd ready`. → seed fabro-66bc**
What happened: the planner's first call, `sd ready --assignee fabro --limit 200` (seq 29–31), returned 27,676 bytes / 200 seeds; only the first line (fabro-5656, top of the High list) was ever used. That one output is the single largest token block in the run's most expensive stage (planner = 43% of cost, biggest input of any stage).
Change: `.fabro/workflows/develop/prompts/planner.md` (PROJECT_FACTS sd-command table) — replace the blanket `--limit 200` forms with a priority-sorted top-N invocation (the 200-limit exists only to defeat the default-50 truncation, fabro-c16d; top-N preserves that intent).
Effect: ~90% cut of the planner's dominant tool output on every develop run; direct reduction of the 43%-of-cost stage.

**2. Expose `current_seed_id` in the `fabro_runs_list` projection. → seed fabro-9372**
What happened: the in-flight check output (seq 39) lists 10 runs with the identical generic goal string and no seed-id field; the planner spent a multi-step reasoning chain (seq 42, ~200-word trace) disambiguating terminal/non-terminal + PR states by hand, and its journal had to note the workaround. The prompt's journal-grep fallback was never exercised only because no collision existed this time.
Change: engine projection feeding `fabro_runs_list` (fabro-9372).
Effect: the in-flight guard becomes a mechanical set-membership test; eliminates the double-pick failure class (fabro-22e4) instead of relying on planner diligence.

**3. Make PR-body generation non-strict JSON. → seed fabro-41b1**
What happened: worker log 20:33:03 — `PR content structured generation failed; retrying once without strict JSON output` (model returned non-JSON). The retry masked it, so PR #159 exists, but every run pays a failed call + retry latency in the postlude.
Change: PR postlude generation path (pull_request pipeline) — drop the strict-JSON parse, accept freeform text (fabro-41b1).
Effect: one fewer failed LLM call per run; postlude stops depending on a retry that hides the real failure mode.

**4. Backfill `pull_request.state` for terminal-but-unmerged runs. → seed fabro-4bb7**
What happened: run 01M2K8TECWAHRR79PQ63V2C0ZP appears in seq 39 as terminal-succeeded with `"state": null` for PR 157 — the exact ambiguous case the planner had to reason around and journal ("backfill pending per fabro-4bb7").
Change: `fabro_runs_list` projection backfill (fabro-4bb7).
Effect: the in-flight rules stop needing status arithmetic to interpret null PR states; removes a recurring per-run journal observation.

**5. Un-hardcode `main` from the just-landed implementer preflight; put the merge-target branch in PROJECT_FACTS. → new-seed justification: no existing seed adds the merge-target branch to *develop's* PROJECT_FACTS (fabro-1128 does it for the revisor prompt only).**
What happened: this run's own deliverable (the fabro-5656 edit to `.fabro/workflows/develop/prompts/implementer.md`) bakes `git fetch origin main … (base branch is main)` into the prompt, while the planner had to spend a probe (`git remote show origin`, bundled into the claim call, seq 49–50) to learn that. The prompt's own PROJECT_FACTS preamble says repo facts live in `project-facts.md`, never in prompts ("a stale value here is loop friction").
Change: add a "merge-target branch: main" line to `.fabro/workflows/develop/prompts/project-facts.md` and reference it from the step-1 preflight instead of the literal.
Effect: removes one planner probe per claim; the preflight can't silently go stale if the merge target ever changes.

**6. Gate the implementer's `sd show` re-fetch on brief quality. → seed fabro-4881** (fabro-a67f is the broader duplicate)
What happened: implementer's first tool call (seq 75–77) re-fetched the full seed body — byte-identical to what the planner had already distilled into `current_seed_brief`. One LLM turn + 1.2 KB for zero new information.
Change: `.fabro/workflows/develop/prompts/implementer.md` step 1 — make the re-fetch conditional on the brief being thin (fabro-4881).
Effect: one fewer turn per implementer pass (~4–5 s first-token latency plus tokens, every run).

**7. Downgrade the by-design allow-key-absence warn to info. → seed fabro-8275**
What happened: worker log 20:31:54 — `WARN … preamble_allow_keys entry absent from context node=implementer key=output.gate_known_bug_hits`. That key only exists after a gatebounce; on a green run its absence is the expected state, so the warn is noise that drowns real signal in an otherwise clean 8-line log.
Change: `lib/components/fabro-workflow` fidelity/lifecycle logging (fabro-8275).
Effect: warn-level log means something again; easier triage of future failures.

**8. Silence the `.codex/instructions.md` not-found probe. → new-seed justification: no existing seed covers the session-initializer file probe (fabro-8275 covers only the allow-key warn).**
What happened: 6 ERROR lines in the worker log (2 per agent session × planner/implementer/reviewer) — `File "/workspace/fabro/.codex/instructions.md" was not found` during session init. Harmless (the file simply doesn't exist in this repo) but it's the majority of this run's error-level output.
Change: session-initializer memory probing (engine, `coding_session_initialize`) — probe conditionally or log at debug/info.
Effect: error-level logs reflect real errors; per-run log noise drops from 6 errors to 0 on green runs.

## Non-issues verified this run (no change recommended)
- Evidence pipe worked end-to-end: the 6.5 KB churn-only capture rendered inline to the reviewer (no blob detour), and the reviewer verified with a single 67 ms grep — `preamble_inline_max_kb=16` is correctly sized here.
- Gate correctly self-classified "no crates touched" (3.6 s tester); `just verify implementer` exited in 117 ms — the config-only cost tier (this run's implementer journal) performed as designed.
- Prompt-accretion pressure is real (this run added a ~1.1 KB paragraph to implementer.md; step 4 is already ~2.7 KB) — fabro-7b2a covers footnoting step 4; if pursued, extend it to the new step-1 paragraph via the same justification pattern as item 5.

What I could not inspect: PR #159's rendered body content and the merge outcome (GitHub-side, not in run events), and the full 200-seed tracker beyond what the truncated listing showed.
