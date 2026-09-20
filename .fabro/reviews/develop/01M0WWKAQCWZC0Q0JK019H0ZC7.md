# Improve review — run 01M0WWKAQCWZC0Q0JK019H0ZC7

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (6.0 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-04 22:15+0000 by revisor `fabro_ask`

---

All facts below are from this run's events, worker logs, and journal (run `01M0WWKAQCWZC0Q0JK019H0ZC7`); baseline: 6/6 stages green, 293s wall / 248s active, $0.271 total, zero retries.

## 1. Fix the PR postlude — ~50s of dead time and two guaranteed failures after the graph finishes
**What happened (from run events + worker logs):** `exit` completed at 16:43:46 (seq 253) but `run.completed` landed at 16:44:39 (seq 259). The 53s in between: PR-body LLM generation **failed** ("No object generated: Failed to parse response as JSON", warn 16:44:33) and fell back to a skeleton, then auto-merge **failed** ("Auto merge is not allowed for this repository", warn 16:44:35; PR #3 created seq 256) — even though the run spec sets `pull_request.auto_merge=true`. Note the failure log names `model=glm-5.3` while the spec configures `zai:glm-4.7` for PR generation — the configured model isn't being used. That's 18% of wall time plus two warn-level failures on every run.
**Change:** run-settings `pull_request` block — set `auto_merge=false` (or enable auto-merge on the GitHub repo), and disable or repair the PR-body LLM call (the deterministic fallback ships anyway; also reconcile the model mismatch).
**Expected effect:** terminal state ~50s sooner per run; two recurring failures disappear.

## 2. Stop blob-ref'ing the evidence capture: `preamble_budget_kb` 24 → 32 (graph attr in `workflow.fabro`)
**What happened (from run events, reviewer prompt seq 218):** the 22.9 KB evidence capture was replaced by a blob ref + preview because it plus the tester output exceeded the 24 KB aggregate budget, forcing the reviewer through a tool round-trip (reviewer `read_file` invoked; its journal: "Evidence arrived as a blob ref; read it in full before judging"). The graph's own comment says the 12→24 raise was meant to fix this; it didn't — the budget is aggregate. Context window peaked at **1.6% of 1M** (reviewer context_window event seq 233): the window is not the constraint, and per the graph comments this same detour caused a wrongful rejection in run 01M0SS23MJ.
**Change:** `preamble_budget_kb=32` (or make the budget per-value so one large capture can't be evicted by small ones).
**Expected effect:** evidence arrives inline; one fewer tool call per review and the "Verification blocked on an unread blob" failure class is closed.

## 3. Evidence diff context `-U1` → `-U3` in `.fabro/workflows/develop/scripts/evidence.nu`
**What happened (from the journal, reviewer@1):** "The -U1 diff context omits where start/last are computed in run(); trusted the green gate plus the pinning tests … rather than re-deriving the range math." The reviewer approved without seeing the exact code the seed is about (range intersection → sum).
**Change:** bump the single `git diff -U1` in `evidence.nu` to `-U3`. Size is no objection — the capture is blob-ref'd today anyway (rec #2 makes it inline, still tiny vs the window).
**Expected effect:** the range/validation logic becomes verifiable in-diff; no more approvals-by-trust at the core seam.

## 4. Resolve the implementer's gate-rule contradiction in `.fabro/workflows/develop/prompts/implementer.md` (step 4)
**What happened (from run events):** the prompt forbids running anything that "formats, lints, or runs the full suite" — yet the implementer's own `implementation_summary` ends "gofmt/vet/test all green", and the tester's gate ran in **1.0s with `ok gofib (cached)`** (tester output seq 218), proving the implementer ran the full suite. The rule is standing-violated, and the violation was actually beneficial (cheap deterministic tester).
**Change:** either legalize what happens — "run `go vet` + `go test ./...`; never `just qualitygate`" — or actually enforce; today it just teaches the model that prompt rules are soft.
**Expected effect:** honest role boundary; the tester node becomes the cheap verifier it was designed to be.

## 5. Make checkpoint metadata snapshots asynchronous or branch-point-only (engine checkpoint pipeline)
**What happened (from run events):** every checkpoint ran a synchronous metadata snapshot: 2.30s at init (seq 22), 2.49s after planner (seq 80), 2.63s after evidence (seq 213), etc. ~2.5s × 7 ≈ **17s, ~6% of the 293s run**, on a linear zero-retry path where nothing ever resumes from them.
**Change:** snapshot async (fire-and-forget commit) or only on failure/graph-branch nodes (tester/evidence/reviewer exits, not the happy path).
**Expected effect:** ~15–17s shaved per seed cycle; compounds on multi-seed runs.

## 6. Stop the implementer from batching parallel edits to the same file (prompt note in `prompts/implementer.md` + engine guard)
**What happened (from worker logs):** three warns — `write_locks: concurrent write to the same file in one batch; serializing` for `/workspace/fabro/main.go` (16:40:46, 16:41:05) and `README.md` (16:42:28). The implementer issued parallel write calls to one file and the runtime had to serialize them; ordering is luck, and the warnings are noise on every run.
**Change:** one line in the implementer prompt ("issue edits to the same file sequentially; parallelize only across different files"), and/or have the agent runtime coalesce/reject batched same-file writes.
**Expected effect:** no racy same-file writes; three warnings per implementer pass disappear.

## 7. Fix `files_touched` attribution for the implementer stage (engine, node outcome metadata)
**What happened (from checkpoint seq 194 vs stage outcome):** the implementer's `files_touched` lists only `README.md` and `main.go`, but the same stage's diff contains `fib_test.go` (+134 lines: TestRunSum, TestRunSumSeedConflict, TestRunSumVersionPrecedence). The metadata undercounts the stage's actual footprint by a third.
**Change:** derive `files_touched` from the stage diff, not from a subset of tool calls.
**Expected effect:** per-stage file attribution is trustworthy for audits, cost dashboards, and the improve loop.

## 8. Fix the `sd ready`/`sd list` rule in `.fabro/workflows/develop/prompts/planner.md`
**What happened (from run events, planner reasoning seq 67 + tool seq 68):** the prompt says "If `sd ready` answers the question, do NOT also run `sd list`" — yet the planner ran `sd list --format json` anyway, correctly: the goal condition "no other open product seed remains" requires seeing **blocked-but-open** seeds, which `sd ready` (unblocked only) cannot answer. The rule contradicts the goal.
**Change:** reword the sd table: `sd list` is expected when the goal requires confirming no other open seeds exist, or when `sd ready` shows a single candidate.
**Expected effect:** removes a prompt instruction that must be disobeyed to do the job right; guards against a future planner routing "Tracker empty" while blocked seeds remain (the deterministic closeout check is the backstop, but the planner shouldn't lean on it).

**What I could not verify:** the exact set of implementer shell commands (e.g., whether it ran `sd show` per step 1) — I inspected the stage prompts, diffs, usage, and warn logs, not every tool call in seq 100–196; and whether `files_touched` misses shell-written files specifically or something else (rec #7 stands on the observed discrepancy either way).
