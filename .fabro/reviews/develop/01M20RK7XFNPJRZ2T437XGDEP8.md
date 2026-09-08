# Improve review — run 01M20RK7XFNPJRZ2T437XGDEP8

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (4.4 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-08 15:11+0000 by revisor `fabro_ask`

---

All recommendations below are grounded in this run's events, journals, logs, and the billing/timing record (run total: 4m07s wall, $0.2235; planner $0.073/59.7s, implementer $0.108/104.8s, reviewer $0.042/42.8s, tester 4.3s, evidence 0.4s, closeout 0.3s — from run events seq 276 and stage outcomes). The run itself was healthy — first-pass gate green, first-pass approval, clean exit, PR #68 — so these target friction that actually fired, ordered by expected impact.

## 1. The in-flight PR guard is 100% inert — the prompt mandates a command that cannot exist (correctness bug, every run)
- **What happened:** planner events seq 45–47: `gh pr list --state open` → `/bin/bash: gh: command not found` → degraded to "no in-flight PRs". The toolchain Dockerfile deliberately removed `gh` (ADR-0019 revert, per the image spec in run events seq 1), so this check fails on **every** run; the double-pick protection (fabro-22e4) is silently disabled. The planner journal says exactly this and names the fix.
- **Change:** land open seed **fabro-06e0** part 2 — rewrite step 4 (IN-FLIGHT PR CHECK) in `.fabro/workflows/develop/prompts/planner.md` to use `fabro_runs_list` (open/non-terminal linked PRs → extract seed ids from run goals), and delete the gh fail-open clause.
- **Expected effect:** removes one doomed tool call + LLM round per planner pass (~$0.017), and — the real win — restores the guard that prevents re-implementing a seed whose PR is already sitting in the gate. This run claimed fabro-16ff with zero open-PR visibility.

## 2. Ship the evidence capture inline — the reviewer burned a tool round-trip on a 12.4 KB blob ref
- **What happened:** the reviewer's own journal painpoint: the capture arrived as a blob ref with a ~300-byte preview, forcing a `read_file` detour before judging (reviewer tool_time 247 ms + one extra model round inside its 42.0s inference). The fix pattern already exists in this graph: the `gatebounce` node's `output_schema` delivers script stdout inline under `output.gate_known_bug_hits` (documented in mx-2eb2e0, proven in the implementer preamble).
- **Change:** give the **`evidence` node** in `.fabro/workflows/develop/workflow.fabro` an `output_schema` (e.g. `@schemas/evidence-capture.schema.json`, capture as a JSON string/structured fields) and declare it in `context_allow_keys`; swap the reviewer's `preamble_allow_keys` entry from `command.output` to `output.evidence`. Complements open seeds fabro-9ef9 / fabro-c1bb (dedupe the ~2.3 KB brief-vs-capture spec repetition).
- **Expected effect:** one model round-trip and one tool call removed per review (~5–10 s), and the `verification_blocked`-on-blob failure mode disappears. 12.4 KB fits the reviewer's 16 KB inline ceiling once the duplicated spec is dropped.

## 3. Enforce the journal/routing JSON shape in the schema — the planner emitted a malformed JSON and self-corrected by doubling its output
- **What happened:** planner final response (event seq 77): first JSON object contained invalid syntax (`...["none"]] && "observation-placeholder"`), then "Correction — valid JSON journal below:" and a full second object. The engine accepted the second, but 1,878 output tokens were billed for the stage — roughly a third re-emitting the brief — and on a worse day the parser latches the first object and the journal is garbage.
- **Change:** extend `output_schema` on the planner/implementer/reviewer nodes (or the shared routing schema) to constrain `context_updates.journal` to `{painpoints: [{text: string}], observations: [string]}` — open seeds fabro-017f / fabro-270c already describe this.
- **Expected effect:** schema validation triggers a structured `output_retries` re-ask instead of free-text self-correction; ~30% planner output-token waste eliminated; journal payload becomes deterministic for downstream mining.

## 4. Kill the planner reconnaissance tax: 22.5 KB firehose + dead `context_read`
- **What happened:** seq 31–34: two `context_read` calls at pass start, one erroring (`unknown context key 'current_seed_id'` — it was visit 1, nothing to read); seq 38–40: `sd ready --assignee fabro --limit 200` returned **168 seeds / 22.5 KB, stdout_truncated: true**, pushing conversation tokens 3.3k → 13.3k. The planner only needed the top of a priority-sorted list (it took candidate #1).
- **Change:** in `.fabro/workflows/develop/prompts/planner.md` step 1: (a) add one line — "visit 1 with no `current_seed_id` in `## Context` = fresh cycle; go straight to `sd ready`" (kills the dead context reads); (b) land open seed **fabro-c3b4** — top-N candidate view instead of the full firehose (keep `--limit 200` semantics for the goal fast-path via `sd show`).
- **Expected effect:** ~10 KB less conversation per planner pass and 2 fewer tool rounds (~$0.02, ~8 s of 59.7 s); less distraction-induced mispicking risk.

## 5. Fix the "Pipeline progress" denominator — agents are told the wrong loop position
- **What happened:** implementer prompt (seq 88): "Pipeline progress: **0 of 7** stages completed" right after planner finished; reviewer prompt: "**2 of 7**" when five stages were done. Open seeds fabro-a0e3 / fabro-45bf / fabro-38f4 all describe this denominator bug.
- **Change:** engine preamble renderer — compute progress from unique completed nodes, not stage-section count.
- **Expected effect:** stage prompts stop contradicting reality; zero cost; removes a standing confusion input for every agent stage (relevant on bounce cycles where knowing "this is implementer@2" matters).

## 6. Gate the implementer's `sd show` re-fetch on brief quality — costliest stage re-pulled a spec it already had
- **What happened:** implementer (48% of run cost, 100.4 s inference for a two-file markdown edit) opened with `sd show fabro-16ff` (seq 98) even though the planner's brief already carried the complete distilled acceptance criteria and the delta analysis. Open seeds fabro-4881 / fabro-a67f cover exactly this.
- **Change:** in `.fabro/workflows/develop/prompts/implementer.md` step 1: re-fetch via `sd show` ONLY when the brief is thin (missing acceptance criteria, verification-only, or carrying review feedback).
- **Expected effect:** one fewer tool call + LLM round per implementer pass (~$0.017); on this run the brief was demonstrably sufficient — the implementer's PASS/FAIL lines map 1:1 to the brief's three criteria.

## 7. Silence the expected-absence WARN — it's the only warning in the run's log
- **What happened:** the run's sole warn-level log line (15:02:29): `preamble_allow_keys entry absent from context node=implementer key=output.gate_known_bug_hits`. That key exists only after a gate-red bounce; on a first visit (this whole run) its absence is by design. Open seed fabro-8275.
- **Change:** make the fidelity lint visit/condition-aware (or downgrade to info) in the engine's `lifecycle::fidelity` check.
- **Expected effect:** warn+ logs become signal again — a real preamble-contract regression won't drown in one expected line per green run.

**What already worked and needs no change (evidence):** the gate's touched-crates fast path (`just qualitygate` → "no crates touched" → fmt-only, 4.3 s vs the 20 m timeout); the churn-only evidence path (seed-work=0, loop-work diff section reviewable); closeout's stdin-scoped `sd close` (closed exactly fabro-16ff); reasoning_effort=low across all three agent stages ($0.22 total, 3,011 reasoning tokens run-wide — the low-effort tuning is paying off).

Sources: run events (seq 1–290), stage journals via checkpoint diffs (`.fabro/journal/01M20RK7XFNPJRZ2T437XGDEP8.jsonl`), worker warn log, and the workflow graph/prompt definitions embedded in the run spec.
