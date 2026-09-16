# Improve review — run 01M2N6MQPYMGSR2GF1VCGFE6RZ

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (4.7 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-16 13:41+0000 by revisor `fabro_ask`

---

All recommendations below are grounded in this run's events, stage transcripts, gate output, journal, worker log, and per-stage cost data (run total: 4 m 43 s wall, $0.244 LLM cost, 216.8 s inference / 24.6 s tool time; planner $0.117 = 48 %, implementer $0.088 = 36 %, reviewer $0.040 = 16 %). Seed ids verified against the live tracker via the planner's `sd ready` listing (run events, seq 30–31).

## Ordered by expected impact

**1. Cap the planner's `sd ready` firehose — seed fabro-66bc**
What happened: the planner's first call, `sd ready --assignee fabro --limit 200`, returned **200 issues = 27,951 bytes of stdout** injected into context (events seq 29–31); it used only the top entry (`fabro-b95d`). The planner is the run's costliest stage ($0.117, 75.7 s inference, 35,114 input tokens — the largest of any stage).
Change: `.fabro/workflows/develop/prompts/planner.md` + the PROJECT_FACTS sd-command table — replace `--limit 200` with a priority-sorted top-N (~10), keeping the anti-truncation guarantee of fabro-c16d server-side.
Effect: ~25 KB less context on every planner pass — a direct cut into the dominant per-run cost.

**2. Persist implementer proof transcripts so the reviewer stops re-deriving them — new-seed justification: fabro-f759 scopes per-criterion check outputs to verification-only runs; this was a normal implementer run, so no existing seed covers it.**
What happened: the implementer built temp fixtures (`bad.md`/`good.md` under `mktemp -d`), deleted them, and reported PASS lines as prose only; the capture can't show deleted files, so the reviewer's "verification economy" rule couldn't apply and it **re-ran the entire negative-path proof itself** (2 shell calls: clean-tree lint + its own `nu -c "echo $x"` fixture — reviewer journal observation, seq 201).
Change: extend `.fabro/workflows/develop/scripts/evidence.nu` to include the transcript of the implementer's per-criterion checks (the verify output) in every capture.
Effect: reviewer approves from context; removes the duplicated ~$0.02–0.04 and 30–45 s of re-verification per run.

**3. Fix the pipeline-progress header miscounts — seed fabro-9e8b (this run is fresh counter-evidence for its basis)**
What happened: the implementer prompt read `Pipeline progress: 0 of 7 stages completed` *after* planner completed (seq 92); the reviewer prompt read `2 of 7` with 4 non-meta stages completed (run stages section).
Change: engine progress projection (lib/, per the seed) — count completed stages at prompt-render time, not run creation.
Effect: honest progress in every stage prompt and squash trailer; `6 of 7` stops being termination-only.

**4. Stop the newline collapse in the `## Current context` table — new-seed justification: fabro-9e49 targets planner-side emission, but the planner DID emit `\n`-bulleted criteria (event seq 81) and the engine still rendered them as one ` / `-separated line in the implementer prompt (seq 92) — the bug is the pipe-table rendering, which no open seed covers.**
Change: engine preamble renderer — emit multi-line context values (at least `current_seed_brief`) outside the pipe-table, or preserve newlines in cells.
Effect: implementer/reviewer read acceptance criteria as checkable bullets, which is the entire point of the bulleted-brief rule; today they parse a ` / -` mush.

**5. Drop the duplicate evidence render in the reviewer preamble — seed fabro-7fba**
What happened: the reviewer prompt carried the capture **twice** — the `## Stage: evidence` section (fidelity `summary:high`, opening with "33 lines omitted") *and* `command.output` (4,071 bytes) via `preamble_allow_keys`.
Change: one line in `.fabro/workflows/develop/workflow.fabro`: add `evidence` to the reviewer node's `preamble_stages_ignore`.
Effect: one canonical copy of the capture, smaller reviewer preamble (reviewer input was 18,782 tokens).

**6. Inline planner probe findings into the brief — seed fabro-fb19**
What happened: the planner `cat`'d `prompt-lint.nu` in full (seq 52–54) and discovered `lint-files` covers only conductor/develop/revisor — then the implementer re-read the same script in its own session anyway (its 8 shell calls include that read) because the brief cited the fact but not the file's relevant content.
Change: planner prompt step 6 — briefs must inline load-bearing facts already probed (existing checks 1–3, glob scope, lint-files coverage).
Effect: fewer implementer read calls and less inference re-deriving what the planner already knew.

**7. Make `ml record` mandatory for reusable tricks, not just near-misses — seed fabro-ee2c**
What happened: the implementer's journal observation names a genuinely reusable pattern ("extract lint checks into self-test-callable functions; invoke on temp fixtures") yet `lesson_capture` answered `nothing durable — skipped` — the trick survives only in this run's journal, which nothing re-reads.
Change: `.fabro/workflows/develop/prompts/implementer.md` lesson-capture section — an observation naming a reusable pattern requires `ml record` + mx-id.
Effect: durable expertise capture instead of per-run journal archaeology.

**8. Validate proof mechanics when seeds are filed/distilled — seed fabro-7773 (extend its lint)**
What happened: seed fabro-b95d's negative-path criterion ("temp copy of a prompt in a scratch dir") was **unexecutable** — prompt-lint globs only the repo tree, so a scratch-dir file is never scanned; the implementer discovered this mid-flight and invented the function-extraction workaround (implementer journal observation).
Change: add to the seed-contradiction lint (fabro-7773): verification commands in seed bodies must be runnable within the tool's actual scan scope.
Effect: implementers stop burning cycles on impossible proof recipes.

**9. Clear the error-log noise — seeds fabro-a701 and fabro-8275**
What happened: the worker log's 8 warn+ lines are 7 noise entries — 6 ERRORs for absent optional `.codex/instructions.md` (2 per agent session × 3 stages) plus 1 WARN for the by-design absent `output.gate_known_bug_hits` on a green run.
Change: log absent optional memory files at info (fabro-a701, engine session init); downgrade the allow-key absence warn to info (fabro-8275).
Effect: worker logs signal real failures only — this run had zero real ones hidden among the noise.

**10. Stop the wasted strict-JSON PR-body attempt — seed fabro-41b1**
What happened: worker log 13:36:08 — "PR content structured generation failed … retrying once without strict JSON"; the retry succeeded (PR #183, 13:36:22) after a wasted LLM round-trip.
Change: PR postlude — request non-strict generation from the start.
Effect: one fewer LLM call and ~10–15 s per run at close.

**Not inspected / out of scope:** I could not verify PR #183's merge state (snapshot shows `state: null`, auto-merge enabled), and gate-bounce/cycle-guard paths were never exercised this run (first-pass green), so no recommendations are made for them. A smaller adjacent item: the planner's in-flight check reasoned over 22 runs whose goals are all the identical generic string — seed **fabro-9372** (`current_seed_id` in the `fabro_runs_list` projection) removes that guesswork engine-side.
