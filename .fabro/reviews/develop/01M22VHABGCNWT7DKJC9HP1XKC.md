# Improve review — run 01M22VHABGCNWT7DKJC9HP1XKC

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (15.2 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-09 10:50+0000 by revisor `fabro_ask`

---

All claims below are from this run's events, stage prompts/transcripts, conclusion billing, and the graph spec as submitted (mirrored in `.fabro/workflows/develop/workflow.fabro`, confirmed present in the workspace). Run shape for context: 907 s wall, **$0.482** total, 7/7 stages green first-pass, zero retries — seed `fabro-8bf4`, PR #84. The implementer stage alone: **789 s wall (87%) and $0.396 (82% of cost), of which 516 s was tool time** vs 272 s inference.

## Recommendations, by expected impact

**1. Warm the Rust build cache for run containers — the single biggest lever.**
Evidence: implementer tool time was 516 s (crate-scoped nextest/fmt/clippy on a cold `target/`), yet the tester ran the identical tree's `just qualitygate` in **24 s** — that 490 s delta is pure cold-cache compile cost, paid inside the LLM-billed stage. Change: implement open seed `fabro-fe15` (bake a debug `fabro-cli`/shared cargo cache into the toolchain image, or engine-mount a persistent `target/` across run containers) — file: the inline toolchain Dockerfile in the run environment spec (`.fabro/Dockerfile*`), needs the pending ADR-0019 user approval. Expected effect: implementer wall drops from ~13 min toward ~5–6 min; ~40% off every Rust-seed run's wall clock, independent of prompt behavior.

**2. Raise `preamble_budget_kb` 24 → 32 — the evidence blob detour fired again in this run.**
Evidence: the reviewer node sets `preamble_inline_max_kb=16`, but the 12.9 KB evidence capture was still demoted to a blob ref (`Output (12.9 KB; full value: /tmp/fabro/runtime/blobs/…)` in the reviewer@1 prompt) because the aggregate 24 KB graph budget was exhausted by the tester section + context keys. The reviewer's **only tool call of the pass** (12 ms `read_file`) was spent paging that blob. Change: graph attr `preamble_budget_kb=32` in `.fabro/workflows/develop/workflow.fabro` (open seed `fabro-8d2c`). Expected effect: captures up to ~16 KB render inline; removes one tool round-trip + one LLM turn per review, and removes the "unread blob = verification blocked" failure mode for typical captures.

**3. Give the planner a top-N `sd ready` view instead of the 24 KB firehose.**
Evidence: planner@1's `sd ready --assignee fabro --limit 200` returned **179 issues, stdout truncated at 24,175 bytes**; the next LLM call's input jumped 12,414 → 21,261 tokens — ~9 k tokens of Medium/Low/Backlog tail the planner never used (it took the first High item). Change: implement open seed `fabro-c3b4` — pipe through `head -40` (or add an `sd ready --top N` flag), and update step 1 of `.fabro/workflows/develop/prompts/planner.md`. Expected effect: ~9 k input tokens + paging latency removed from every planner pass; this run's planner was 34.8 s inference / $0.057 — expect roughly half that.

**4. Scope per-stage memory — full `AGENTS.md` rides into all three agent sessions.**
Evidence: `agent.memory.loaded` shows the identical **23,967-byte `AGENTS.md`** (~6 k tokens) in planner@1, implementer@1, and reviewer@1; the planner never touches the Rust workspace it describes. Related: `cache_write_tokens=0` all run and each stage's first LLM call starts cache-cold (planner call 1: 12.4 k input, 0 cache-read) — the system+tools+memory prefix is re-paid per stage (open seed `fabro-944d`). Change: per-node memory scoping (`fabro-9588`) — tracker/loop sections for planner, Rust conventions for implementer/reviewer; engine-side, shared prompt-cache prefix (`fabro-944d`) compounds it. Expected effect: ~12–18 k input tokens saved per run and less cross-role distraction; small dollars, but every run pays it.

**5. Give non-blocking review findings a durable home.**
Evidence: the reviewer's one substantive observation — `unemitted_allow_keys` matches raw keys while `read_context_key` tolerates `context.`-prefixed forms, "key-normalization divergence is a future watch item" — exists **only as journal prose** (`reviewer@1` journal, and the run-diff `.fabro/journal/…jsonl`). Nothing converts it into a tracker seed; it survives only if the improve workflow scans journals. Change: add a `review_notes` context key to the reviewer's `context_allow_keys` and have `closeout.nu` file non-blocking notes as Low-priority seeds (open seed `fabro-89dd`), files: `.fabro/workflows/develop/workflow.fabro` + `scripts/closeout.nu`. Expected effect: real latent-bug findings become tracked work automatically instead of depending on a second scan loop.

**6. Stop restating acceptance bullets in every artifact.**
Evidence: the 1.6 KB `current_seed_brief` appears verbatim in the planner output, implementer prompt, and reviewer prompt; the implementer's `implementation_summary` (1.4 KB) then restates each bullet inside its PASS lines, and the reviewer's 3.4 KB response restates them a third time — ~3 KB of duplicated criterion text per downstream stage. Change: implement open seeds `fabro-4cd8`/`fabro-ad1b` — cite bullets by index (`PASS #2: context.rs envelope_lint_* tests`) — prompt edits in `prompts/implementer.md` and `prompts/reviewer.md`. Expected effect: 1.5–2 KB less preamble per stage; zero-risk.

## What I could not ground

**Error handling:** this run had zero failures, retries, gate-reds, or output-schema retries, so no failure-path data exists to improve. The nearest grounded items: (a) the implementer's two compile iterations (fn-pointer vs clippy-form mismatch) are now captured as `mx-7eb5e7`, so the existing `ml prime` flow will surface them next time — no change warranted from this run; (b) the only degradation that actually fired was the blob demotion (covered in #2). I did not inspect PR #84's post-closeout merge state (still `null` at last event) or the runner logs.
