# Improve review — run 01M2VH6A91ESRYJ0GGWQJ24CMQ

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (23.2 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-19 01:18+0000 by revisor `fabro_ask`

---

All evidence below is from run 01M2VH6A91ESRYJ0GGWQJ24CMQ (seed `fabro-meta-c9f2`, all 9 stages first-pass green, 22.8 min wall, $1.417 total) — stage timings/usage and transcripts from run events, friction items from the stage journals, seed existence verified against workspace file `.seeds/issues.jsonl`.

**1. Ship the warmed toolchain image — seed `fabro-c643` (Priority 1, already consolidated + user-GO, in-flight via PR 243).**
What happened: the implementer stage was 1,102 s wall / $1.014 — 72% of run cost — of which 715.6 s was tool time dominated by cargo build/clippy/nextest; the tester gate was green in only 55.4 s purely because the implementer left the cache warm (from run events, stage timings). Change: land the multi-arm `.fabro/Dockerfile.toolchain` rebuild (cargo-chef layers + mold, per the seed's consolidation note). Expected effect: 4–7 min off every develop run's wall — the single largest lever this run exposes; also de-risks the ~15 min cold-gate case.

**2. Raise the reviewer's inline evidence ceiling to ≥40 KB — seed `fabro-cf3e`.**
What happened: this run's evidence capture was 38,917 B (marker said "39.4 KB; full value: …"), above the reviewer node's `preamble_inline_max_kb=16`, so it was demoted to a blob ref; the reviewer burned 5 shell calls + 1 read_file paging it before approving (from run events, reviewer@1 tool stats) — the exact friction family the run's own seed fixes on the line-cap axis. Change: `.fabro/workflows/develop/workflow.fabro`, reviewer node, `preamble_inline_max_kb` 16 → ≥40 (the seed's 2026-09-16 update already amends the stale 32 target; this run is fresh confirming evidence). Expected effect: evidence arrives inline, ~6 tool round-trips and the unread-blob rejection class disappear from every review.

**3. Require match-counting before pattern-wide transforms — seed `fabro-a158`.**
What happened: the implementer's perl signature-adaptation pass missed two test call sites (`preamble.rs` ~1780/1829), caught only at nextest compile as E0061 — one wasted compile cycle inside an 18-minute stage (implementer journal painpoint, this run id). Change: `.fabro/workflows/develop/prompts/implementer.md` step 4 — `rg -c` count before any sed/perl transform; count ≠ intended sites ⇒ line-addressed sed or `edit_file`. Expected effect: signature-ripple misses surface before the expensive compile, cutting recovery round-trips in the dominant stage.

**4. Top-N `sd ready` view instead of the 200-seed firehose — seed `fabro-c3b4`.**
What happened: the planner's first shell call returned 29,226 bytes over 200 seeds with `stdout_truncated: true` (from run events, planner@1 seq 38) — the model saw a silently truncated list and still had to consume ~4k tokens of queue to pick one seed; planner was the second-costliest stage ($0.304, 135.9 s for a claim + brief). Change: `.fabro/workflows/develop/prompts/planner.md` steps 1–2 — top-N first (`sd ready --first 10` / `--priority high`), full listing only as fallback. Expected effect: smaller planner context every run and no silent omission of higher-priority candidates hidden in the truncated middle.

**5. New seed — closeout deploy-gate note for engine-path seeds.** Justification: no tracker seed covers it — greps of `.seeds/issues.jsonl` for redeploy/deploy-velocity return nothing (mx-6b7121 is an expertise record, not a seed; `fabro-1409` is PR cost tables, `fabro-dc81` is pull_request re-resolution).
What happened: the run's own fix (`preamble_output_max_lines=200`) is inert until the server image redeploys — this run's reviewer still rendered under the old engine, so the acceptance criterion "zero lines omitted in the evidence section" was verifiable only by unit test, never live (reviewer journal painpoint, this run id). Change: `.fabro/workflows/develop/scripts/closeout.nu` — when the seed-work diff touches `lib/components/fabro-workflow/**`, append a "deploy-gate: image redeploy required before this fix is observable in-run" line to the closure note/PR body, and file a post-redeploy verification-only check (for this seed: a reviewer evidence section with zero "lines omitted" on a >50-line capture). Expected effect: the next run's planner knows dogfooding is impossible pre-redeploy; no cycle is burned verifying against a half-live engine.

**6. Batch planner reconnaissance into one shell call — seed `fabro-55a7`.**
What happened: the planner made 14 sequential shell probes plus one `fabro_runs_list` across 16 messages (sd ready → runs list → sd show → five separate greps/seds walking `preamble.rs`/`workflow.fabro`/`artifact.rs`), each probe costing an LLM round-trip inside its 125 s inference (from run events, planner@1 transcript). Change: `.fabro/workflows/develop/prompts/planner.md` — chain recon (sd ready + sd show + base greps) into one labeled shell call, per the seed. Expected effect: ~40–60 s and ~$0.06 saved per planning pass and a shorter claim-to-implement window, narrowing the duplicate-claim race.

**7. New seed — `cargo test --no-run` before fmt/clippy in verify.nu.** Justification: no existing seed covers compile-before-lint ordering — `fabro-2f70` is test-file *detection*, `fabro-a1ef` is rebuild-before-re-diagnosis; greps for "no-run" in `.seeds/issues.jsonl` return nothing.
What happened: the E0061 from rec 3 surfaced only at nextest compile, after fmt/clippy had already run — the implementer's own journal painpoint this run suggests exactly this change. Change: `scripts/verify.nu` implementer stage — run `cargo test --no-run -p <touched-crate>` ahead of fmt/clippy so signature-ripple misses surface at the cheapest phase. Expected effect: compile errors stop paying the fmt+clippy toll first; faster, cheaper implementer verification on every Rust seed.

Not recommended for change (evidence they're already working): preflight (4.3 s), claim_check (84 ms), evidence capture (342 ms), closeout (418 ms) — the deterministic nodes cost nothing and the graph's cycle guards/schema teeth never fired (zero retries all run).
