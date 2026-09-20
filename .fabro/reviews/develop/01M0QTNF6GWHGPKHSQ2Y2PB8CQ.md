# Improve review — run 01M0QTNF6GWHGPKHSQ2Y2PB8CQ

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: failed (4.8 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-04 22:28+0000 by revisor `fabro_ask`

---

I've inspected the full event history, stage transcripts, worker log, journal files, and the workflow scripts. Here is what actually happened, and what to change — ordered by expected impact.

**Run shape for context** (from run events): one complete seed cycle (`fabro-8d26`, gofib `-version` flag), 208.9s active / 245.9s wall, $0.181 total. Per stage: planner@1 41s/$0.034 · implementer@1 132s/$0.118 (65% of cost) · tester 1.0s · evidence 0.4s · reviewer 22.7s/$0.013 · planner@2 15.9s. Seed closed, tracker empty, branch pushed — then the run was marked **failed**.

---

**1. Make PR publish failure non-fatal, with a deterministic fallback** *(error handling / UX — highest impact)*
- **What happened:** worker log, 17:34:19: `Pull request creation failed error=LLM generation failed: No object generated: Failed to parse response as JSON: expected value at line 1 column 1` → `failed(publish_failed)`, `total_retries: 0`. Every graph stage succeeded; the seed was closed and `fabro/run/…` was pushed (commit `ee9ca75`); `pull_request: null` so the configured squash auto-merge never ran. A fully green loop is reported as failed.
- **Change:** in the engine's publish path (not the graph): on LLM PR-body generation failure, retry once, then fall back to a deterministic template (title = run title, body = goal + closed seeds + diff stat from the final checkpoint), and report `succeeded` with a publish warning instead of `failed`.
- **Expected effect:** run status matches reality; auto-merge resumes; no human triage of green runs. This is the only defect that made the run a failure.

**2. Raise `preamble_budget_kb` so the reviewer actually sees the diff** *(graph design / correctness)*
- **What happened:** the reviewer ran as handler `prompt` with **zero tools** (tool_time_ms = 0, no tool events), yet its prompt mandates "read the blob with your tools before judging." The 6.2 KB evidence capture (self-budgeted to 6,800 chars in `evidence.nu` to stay under the 8 KB per-value demote threshold) was demoted anyway by the **aggregate** budget — worker log 17:33:44: `preamble values exceed aggregate budget even after demotion… total_bytes=15862 budget=12288`. The reviewer approved from the integrity header + previews and flagged exactly this: "the retry contract then permitted JSON-only output, which blocks the fallback blob read." The graph comment in `workflow.fabro` ("summary:high carries the evidence capture in full") is empirically false this run. The loop's own legitimate data (brief 1.3K + feedback 1.4K + gate log + evidence 6.2K + summaries) sums to ~16 KB.
- **Change:** `.fabro/workflows/develop/workflow.fabro`, graph attr `preamble_budget_kb=12` → **24**. (Optionally also give the reviewer an agent handler with read-only tools as defense-in-depth.)
- **Expected effect:** the complete diff and seed spec reach the reviewer inline; verdicts stop depending on a fallback the node cannot execute. Cost is trivial — reviewer input was 3.4K tokens ($0.013); one wrong approval bouncing a seed costs a full ~3 min / ~$0.15 cycle.

**3. Finish the journal painpoint pipe — it currently writes empty files** *(tooling / self-improvement loop)*
- **What happened:** all 7 `.fabro/journal/*.json` files contain `"data": {}`. `stage-journal.nu`'s own comment admits it: "until HookContext carries context_updates — engine work item 2 — this hook reads nothing from agents." The reviewer's one genuine painpoint survived only because it was pasted into `review_feedback` — against its own output template — and then itself got blob-ref'd at planner@2.
- **Change:** engine work item 2: include the stage's `context_updates` in `FABRO_HOOK_CONTEXT`, and have `stage-journal.nu` copy `journal.painpoints` into `data`. Also add one line to `prompts/reviewer.md`: "Approved JSON contains ONLY `review_verdict`; observations go in `journal.painpoints`."
- **Expected effect:** the improve-workflow that "scans run branches for journals" finally receives data; findings like #2 become machine-readable instead of buried in a verdict field.

**4. Stop the implementer from re-running the whole gate** *(prompting / cost)*
- **What happened:** `prompts/implementer.md` rule 4 says "Do NOT run the full quality gate yourself… a quick smoke check is fine," but the transcript shows it ran `gofmt`, `go vet`, `go test ./...`, plus **four separate `go run` invocations** — then the tester node re-ran `just qualitygate` in 1.03s (cached). The implementer consumed 117.8s inference = 56% of run active time and $0.118 of $0.181.
- **Change:** make rule 4 a bounded instruction: "exactly one smoke command before finishing — `go build ./... && go test -run <the new test> ./...` — never the full suite; the tester node is the gate."
- **Expected effect:** roughly 30–50s and ~20–30% of implementer cost cut per cycle; gate feedback arrives one step earlier regardless.

**5. Bake the toolchain into the runner image** *(environment / wall time)*
- **What happened:** setup events: `mise install` took **54.2s** of the 55.9s prepare phase — ~22% of this 246s run, paid again on every run of a loop designed to repeat per seed.
- **Change:** pre-install the pinned mise tool versions in the `fabro-runner:mise` Dockerfile (or persist the mise cache across sandbox snapshots).
- **Expected effect:** ~50s saved per run — the single largest fixed-latency cut available.

**6. One-file edits sequentially, not as parallel batches** *(tool usage)*
- **What happened:** worker log, 17:31:51: **six** `WARN fabro_agent::write_locks: concurrent write to the same file in one batch; serializing path="/workspace/fabro/fib_test.go"` — the implementer issued ~6 parallel `edit_file` calls against one file, all serialized into round trips anyway.
- **Change:** one line in `prompts/implementer.md` output-hygiene section: "multiple edits to the same file → issue them sequentially, or emit the final content once with `write_file`."
- **Expected effect:** fewer wasted tool round trips; the lock warnings disappear from every run's log.

**7. Trim the per-stage prime ritual and the duplicate tracker query** *(prompting / efficiency)*
- **What happened:** planner and implementer each spent an LLM turn on `sd prime`/`ml prime` (~5 KB of context), and `sd prime`'s close checklist instructs `bun test && bun run lint && bun run typecheck` — a wrong-toolchain instruction for this Go repo sitting in every agent's context. The planner also ran `sd ready --format json` and `sd list --format json` in parallel; both returned the identical single seed.
- **Change:** in `prompts/planner.md`: "`sd ready` alone; only fall back to `sd list` when ready is empty." In `AGENTS.md`: replace the bun checklist with `just qualitygate`, and mark priming as once-per-session rather than per-stage.
- **Expected effect:** 1–2 fewer LLM turns per agent stage (~10–20s and a few cents per cycle) and no misleading bun commands for an agent to blindly follow. Relatedly: `rg` is missing from the image (planner hit `rg: command not found` before recovering with the native grep tool) — adding ripgrep to the image removes a known-dead path agents keep trying.

---

**Not recommend touching:** the tester/evidence split (1.0s + 0.4s, both cheap and correct), the cycle-guard prompt logic (never triggered; no dead cycles occurred), and `preamble_stages_ignore` scoping (it worked — planner@2 ran on 7.9K input tokens precisely because history was scoped away).

Sources: run events and stage transcripts (`fabro_run_events`, `fabro_run_get`), worker warning/error log (`fabro_run_logs`), journal files and scripts read from the workspace (`.fabro/journal/*.json`, `.fabro/workflows/develop/scripts/{stage-journal,evidence}.nu`, graph definition from the run spec).
