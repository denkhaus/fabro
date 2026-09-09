# Improve review — run 01M23EDVNHPGD8QNBWZN8WNTDD

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (36.7 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-09 16:48+0000 by revisor `fabro_ask`

---

Recommendations, ordered by expected impact. Each is grounded in a specific observation from this run (run 01M23EDVNHPGD8QNBWZN8WNTDD, seed fabro-94e8, total 36.7 min / $2.20, implementer stage = 30.1 min / $2.04 = 92.6% of cost).

## 1. Scope the implementer's nextest to test-touched crates only
**Evidence (run events, implementer summary):** the implementer ran the FULL nextest suite on all **five** touched crates — "3370 passed across the five touched crates" — but tests were only added in `fabro-github` and `fabro-server` (`lib/components/fabro-github/src/lib.rs`, `lib/apps/fabro-server/src/server/tests.rs`). `fabro-types`, `fabro-store`, `fabro-workflow` got code changes only. The tester gate then re-ran the identical five-crate commands (gate log in reviewer preamble: fmt + clippy + nextest on the same five) in 227 s. The rule in the prompt ("full crate suite for every crate whose tests the seed touched") was over-applied, and the deterministic gate re-covers everything anyway.
**Change:** `.fabro/workflows/develop/prompts/implementer.md`, step 4 — make the scoping mechanical: "nextest only for crates whose *test files* the seed touched; fmt/clippy for every code-touched crate."
**Expected effect:** cuts a large slice of the implementer's 1,092 s tool time / 30-min wall on every multi-crate Rust seed; gate still verifies deterministically.

## 2. Hard rule against parallel edits to the same file
**Evidence (worker log, 4 warnings):** `concurrent write to the same file in one batch; serializing` for `run_event/mod.rs` (16:10), `event/events.rs` (16:11), `serve.rs` (16:12), `pull_request_supervisor.rs` (16:16). This is exactly the near-miss class the prompt cites from run 01M11P68SHFS (19 warnings, one swallowed loop body) — the prose warning did not stop the behavior.
**Change:** `.fabro/workflows/develop/prompts/implementer.md` — promote to a hard rule ("never two edits to one path in a single tool batch"), or enforce engine-side per open seed fabro-1d94.
**Expected effect:** eliminates the serialization-detour and the swallowed-edit correctness risk on every multi-file seed.

## 3. Stop the planner `sd ready` firehose
**Evidence (event 32):** one `sd ready --assignee fabro --limit 200` poured **25,841 bytes / 191 seed lines** into the planner conversation just to learn that fabro-94e8 is top — ~40% of the planner's 27.5k-token context. Open seed fabro-c3b4 proposes exactly this and its body says "fresh evidence" was still needed.
**Change:** `.fabro/workflows/develop/prompts/planner.md` + the command table in `prompts/project-facts.md` — top-N/priority-filtered first call (`--priority high`-style, ~10 rows), full listing only when top candidates are unclaimable.
**Expected effect:** ~25 KB less context per planning pass on every run; cheaper, less distracted picks.

## 4. Inline the evidence capture for the reviewer
**Evidence (reviewer journal + stage transcript):** the 42,947-byte capture exceeded `preamble_budget_kb=24`, arrived as a blob ref, and the reviewer "had to page `/tmp/fabro/runtime/blobs/6a86853c…json` manually … the preview alone would have been insufficient" — while using 2.5% of its 1M window. Recurring friction (open seeds fabro-8d2c, fabro-cf3e).
**Change:** `.fabro/workflows/develop/workflow.fabro` — raise graph `preamble_budget_kb` 24→48, or reviewer `preamble_inline_max_kb` 16→48.
**Expected effect:** no blob round-trip per review; removes the "Verification blocked" re-capture risk (a full evidence re-cycle) when a reviewer can't or doesn't page the blob.

## 5. Add a crate→path map and available-binaries line to PROJECT_FACTS
**Evidence (events 46, 99):** planner ran `fd` → `fd: command not found`; implementer guessed `lib/foundation/fabro-github` → "No such file or directory" (it lives in `lib/components/`). Both stages burned calls discovering layout the loop already knows. Open seed fabro-3d2d.
**Change:** `.fabro/workflows/develop/prompts/project-facts.md` — add a crate→directory map (github/components, server/apps, types/foundation…) and a "shell has rg/sd/ml/just/nu; no fd" line.
**Expected effect:** removes path/binary probing calls in planner and implementer on every run.

## 6. Give non-blocking review findings a durable channel
**Evidence (reviewer journal):** "staleness_supervisor_skips_clean_and_blocked_run_pull_requests only tests the clean fixture, not blocked" — non-blocking, so it lived only in the journal; the seed was closed at 16:37 and no future planner is bound to fold it into a brief. That test gap is now effectively orphaned. Open seed fabro-89dd.
**Change:** `.fabro/workflows/develop/workflow.fabro` reviewer node — add a `review_notes` context key relayed to closeout/planner.
**Expected effect:** small follow-ups (the blocked fixture, the in-memory-cap persistence note) become actionable instead of evaporating at closeout.

## 7. Stage heartbeat notification
**Evidence (settings + timings):** notifications are only `run.completed`/`run.failed` to `#dev-fabro`; the implementer ran **30 minutes** with zero operator visibility (16:02→16:32). Open seed fabro-b769.
**Change:** run settings terminal notification config — add a long-stage heartbeat (e.g. >10 min).
**Expected effect:** mid-run visibility; faster human intervention on stalls.

## 8. Downgrade the by-design allow-key warning
**Evidence (worker log):** `preamble_allow_keys entry absent … output.gate_known_bug_hits` fired on the green-path implementer start — expected, since that key only exists on gate-red bounces. Open seed fabro-8275.
**Change:** engine (`fabro_workflow::lifecycle::fidelity`) — downgrade to info.
**Expected effect:** one less noise line per green run; keeps warn+ logs meaningful.

Not inspected: the PR-body generation (auto-merge, `zai:glm-4.7`) happened after run completion (last event 16:40:55) and its transcript is outside the run events I could read, so I made no recommendation about it.
