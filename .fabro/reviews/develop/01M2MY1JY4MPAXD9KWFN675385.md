# Improve review — run 01M2MY1JY4MPAXD9KWFN675385

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (20.5 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-16 11:46+0000 by revisor `fabro_ask`

---

All evidence below is from this run's events/checkpoints/logs (run `01M2MY1JY4MPAXD9KWFN675385`, seed `fabro-45bf`, PR #175, 20.3 min wall, $0.482 LLM cost: planner $0.210/132 s, implementer $0.232/701 s incl. 556 s tool time, tester 340 s, reviewer $0.041/26 s) and seed statuses verified against workspace file `.seeds/issues.jsonl`. Ordered by expected impact.

## 1. Make the evidence capture survive the summary:high renderer cap — seed `fabro-meta-c9f2` (open, High)
- **What happened:** the reviewer's prompt rendered the evidence section as "(115 lines omitted)" mid-diff with no blob-ref marker (reviewer stage prompt, seq 289+); the whole `sessions.rs` half of the diff was invisible. The reviewer recovered via a read-only `git diff 51c8a94..HEAD` (reviewer journal painpoint, checkpoint seq 318) and journaled exactly this gap.
- **Change:** in the evidence delivery path (`nu .fabro/workflows/develop/scripts/evidence.nu` output shape, or the engine stage-renderer tail_lines cap in `lib/components/fabro-workflow/src/handler/llm/preamble.rs`), diff-first ordering within the rendered window and a blob-ref marker whenever anything is omitted (complements `fabro-cf3e`'s 16→32 KB inline raise, which doesn't fix the line cap).
- **Expected effect:** reviews verify from context in one cycle on ~120-line diffs; eliminates the git-recovery detour and the "Verification blocked" re-cycle risk this class caused in prior runs.

## 2. Give the run container a warm Rust build — new-seed justification: `fabro-cfd6` covers only the CI dogfood-gate image, no open seed covers the run-container toolchain image or a persistent build cache
- **What happened:** implementer tool time 556 s (fmt/clippy/nextest in `just verify implementer`) plus tester `just qualitygate` 340 s ≈ 15 of the run's 20 minutes were Rust compilation in a fresh container (environment `lifecycle.preserve=false`, image `ghcr.io/denkhaus/fabro-toolchain:99c855a689b3`, no cross-run cache).
- **Change:** bake cargo-chef dependency layers into the toolchain image (or mount a persistent sccache volume via `run.environment` in the develop workflow settings).
- **Expected effect:** roughly 4–7 min off every develop run's wall time — the single largest wall-time lever visible in this run.

## 3. Stop evidence overwriting the gate log — seed `fabro-9ef9` (open)
- **What happened:** tester wrote `command.output = blob 9d94ab…` (checkpoint seq 278) and evidence silently replaced it with `blob f5850b…` (seq 286) — the exact last-writer-wins clobber, harmless only because the gate was green.
- **Change:** key the evidence command node to `evidence.output` via command-node keying in `.fabro/workflows/develop/workflow.fabro` + `evidence.nu`.
- **Expected effect:** on any gate-red path that reaches review, the gate failure tail stays readable from context instead of being clobbered.

## 4. Port the `rg -rn` footgun line to the planner prompt — seed `fabro-6997` (open)
- **What happened:** the planner ran `rg -rn "Pipeline progress" lib/` (event seq 43); `-r n` replaced every match with literal `n` (`n: {completed_count}…`). It then burned ~5 more calls across seq 49–97 hypothesizing "rg config with a --replace setting" before falling back to plain `grep -rn` (seq 95). Planner was 43% of run cost; this detour was the biggest single waste in it.
- **Change:** add the two one-liners (fs_hide glob returns empty; never `rg -rn`) to `.fabro/workflows/develop/prompts/planner.md` — they exist today only in `implementer.md`.
- **Expected effect:** −4–6 planner tool calls and ~60–90 s inference per run (~$0.03–0.05), and no corrupted probe outputs feeding the brief.

## 5. Top-N the planner's `sd ready` view — seed `fabro-66bc` (open)
- **What happened:** the planner's first call returned 200 ready seeds / ~28 KB with `stdout_truncated: true` (seq 30–31), yet it picked the first High-priority line (`fabro-45bf`).
- **Change:** switch the planner command table to a top-N priority-sorted `sd ready` invocation (keep the assignee filter).
- **Expected effect:** smaller planner context every run — cheaper turns, less truncation noise in the transcript.

## 6. Per-role PROJECT_FACTS sd table — seed `fabro-52b4` (open)
- **What happened:** the reviewer's prompt carried the full 6-row sd write-command table (claim/close/re-status forms) for a role that is tracker-read-only by policy; it used exactly 2 read-only git calls all pass.
- **Change:** factor the PROJECT_FACTS sd-command include per role in `.fabro/workflows/develop/prompts/` (reviewer: none; implementer: `sd show` only).
- **Expected effect:** ~1–2 KB less prompt per stage and no forbidden-command forms advertised to roles that must not use them.

## 7. Downgrade missing-optional-memory logs + non-strict PR body — seeds `fabro-a701` and `fabro-41b1` (both open)
- **What happened (run logs):** six ERROR lines for absent optional `.codex/instructions.md` (one pair per agent session init — 6 of the 28 warn+ lines were this noise), and `PR content structured generation failed; retrying once without strict JSON` at 11:21:47 in the postlude.
- **Change:** log absent optional memory files at info (`fabro-a701`, agent session init); make PR-body generation non-strict-first (`fabro-41b1`, PR postlude).
- **Expected effect:** a warn+ log view that only shows real problems, and no masked-retry latency on every run's PR creation.

## 8. edit_file closest-match hint — seed `fabro-e33d` (open)
- **What happened:** implementer `edit_file` 6 calls, 1 error (agent tool stats, seq ~270); the provider also warned 16× "does not support the tool result error flag", so the error surfaced only as stdout text (that protocol gap is `fabro-b09c`, open).
- **Change:** include closest-match/whitespace-divergence line in the `edit_file` error (`lib/components/fabro-agent/src/tools.rs`) plus the `sed -n` old_string sourcing line in `implementer.md`.
- **Expected effect:** one fewer diagnostic round per mismatch; fewer false "transient matcher" lessons recorded.

Not inspected / out of scope: the auto-merge squash of PR #175 and the Slack notification happened after run completion (last event 11:44), so their outcomes weren't in scope here; gate internals beyond the captured `GATE GREEN` tail were not inspectable.
