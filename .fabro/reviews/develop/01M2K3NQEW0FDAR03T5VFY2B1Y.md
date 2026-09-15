# Improve review — run 01M2K3NQEW0FDAR03T5VFY2B1Y

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (17.7 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-15 18:43+0000 by revisor `fabro_ask`

---

All evidence is in. Here are the recommendations, ordered by expected impact, each grounded in this run (01M2K3NQEW0FDAR03T5VFY2B1Y, seed fabro-9d2f, PR #153; wall 17.7 min, cost $0.814; implementer alone: 14.5 min / $0.656 = 84% of wall, 81% of cost — from run checkpoints/conclusion).

---

**1. Warm the Rust build cache in run containers — existing seed fabro-fe15 (open, needs-user).**
What happened: implementer tool_time was 531.6 s of 872 s active (61%) — cold fmt/clippy/nextest on two crates; the tester's gate was then green in just 37.8 s only because the implementer pre-warmed it (the crate-scoping policy working as designed). Change: bake cargo-chef/debug-target layers into the toolchain image per fabro-fe15 (`.fabro/Dockerfile*` → `ghcr.io/denkhaus/fabro-toolchain`); it explicitly complements fabro-cfd6 (CI-only) and is parked on ADR-0019 user approval — this run adds the missing run-container evidence. Expected effect: 4–6 min off every Rust seed's implementer stage (~30% of run wall).

**2. Fix verify.nu's blind spot for inline `#[cfg(test)]` tests — NEW SEED.**
What happened: the implementer journaled (verified in the stage checkpoint) that `scripts/verify.nu`'s test-file-touched derivation returned empty although 3 table-driven test cases were written — they live in an inline test module in `src/lifecycle/git.rs`, which the dispatcher doesn't count. Verify degraded to compile-check; only the deterministic gate ran the new tests. One change: in `scripts/verify.nu`, mark a crate test-file-touched when the diff touches `#[cfg(test)]` regions (grep the diff). Expected effect: the implementer-side test signal — the loop's pre-gate safety net — actually fires on every Rust seed using inline test modules (the project convention). New-seed justification: this is a detection defect in closed fabro-c4ce's delivery; open fabro-7f58 only narrows *which* tests run once detected — no seed fixes detection.

**3. Strip mutating fabro tools from the reviewer/implementer nodes — NEW SEED.**
What happened: reviewer@1, event seq 394 — the "read-only by capability and policy" reviewer routed its verdict by calling `fabro_run_interact action=message` **on its own run**, then re-emitted the routing JSON ("verdict already issued and routed. Re-emitting…"). One wasted call, and the tool it reached for exposes start/approve/deny/interrupt/cancel. One change: in `.fabro/workflows/develop/workflow.fabro`, give the reviewer and implementer nodes a per-node tool allowlist using the engine's existing NamedToolAccessPolicy `tools=`/`fabro_tools` attribute (the planner already proves the mechanism — its session registered exactly `fabro_runs_list`). Expected effect: verdicts travel only via the routing JSON; the mutation-channel misuse class disappears. New-seed justification: the mechanism landed in closed work (commit c867a1a8c; planner wiring closed via fabro-c419), but no open seed applies it to the reviewer/implementer nodes — fabro-9588 covers skills/memory scoping only.

**4. Deliver the evidence capture inline — existing seeds fabro-cf3e + fabro-8d2c (both open).**
What happened: this run's capture was 20,245 bytes; the reviewer's prompt rendered it as a blob ref (`Output (20.4 KB; full value: /tmp/sandbox-driver/runtime/blobs/f061…json)`) because `preamble_inline_max_kb=16` on the reviewer node, and 20.4 KB + tester section + context keys also bust the 24 KB graph budget. One change: reviewer node `preamble_inline_max_kb` 16→32 (fabro-cf3e) and graph `preamble_budget_kb` 24→32 (fabro-8d2c) in `workflow.fabro`. Expected effect: ~20 KB captures arrive inline; one fewer tool round trip per review and the standing unread-blob rejection class is gone for this size.

**5. Route the run's own flagged risk into review, and test the retry path — NEW SEED.**
What happened: the implementer's journal observation warns that the new quarantine changes retry semantics (a `Failed{retry_requested:true}` stage passes through `on_checkpoint`, so its half-edit is reverted and the retry restarts from the last checkpoint) and marks it "worth reviewer attention" — but the reviewer never adjudicated it: `journal` is not in the reviewer's `preamble_allow_keys`, the summary didn't repeat it, and the reviewer's observations don't mention it. Approval shipped anyway; the 3-case test table has no retry-jump case. One change: (a) add a 4th case to the table in `lib/components/fabro-workflow/src/lifecycle/git.rs`; (b) in `.fabro/workflows/develop/prompts/implementer.md`, require material semantic-risk observations to be repeated in `implementation_summary`. Expected effect: flagged risks can't escape review; this run's feature gets its missing test. New-seed justification: fabro-89dd routes reviewer→planner non-blocking notes (opposite direction); nothing routes implementer risk flags to the reviewer or adds the retry case.

**6. Top-N `sd ready` view in the planner — existing seed fabro-c3b4 (open).**
What happened: planner's first call returned 27,809 bytes / 200 seed rows into context (event seq 30) to pick one seed; planner burned 26.9k input tokens / $0.083 on tie-breaking a firehose. One change: planner.md — `--first 10`/priority-filtered view with full-listing fallback, plus `created_since` on `fabro_runs_list` (already specced in fabro-c3b4). Expected effect: smaller planner context every pass — modest per run, paid on every run.

**7. Journal the errored tool calls — existing seed fabro-a23f (open).**
What happened: implementer@1 stats show 24 shell calls with **2 errors** (from the stage session stats in run events); neither the painpoints nor observations mention them — I found them only by event archaeology, exactly the failure mode fabro-a23f documents. One change: implementer.md journal section mandates naming errored/timed-out calls even when recovered. Expected effect: friction self-reports on the run where it happens instead of needing manual analysis.

---

What already worked and needs no change: gate green first-try in 37.8 s (implementer pre-warm), closeout 0.2 s, in-flight guard self-excluded cleanly, one-pass approval with inline PASS/FAIL report, and the planner followed the fabro-11d3 tie-break (bug over polish) exactly.

Sources: run events (seq 30, 394–395, stage checkpoints/conclusion), stage journals in the run diff, and workspace file `.seeds/issues.jsonl` for seed status/wording.
