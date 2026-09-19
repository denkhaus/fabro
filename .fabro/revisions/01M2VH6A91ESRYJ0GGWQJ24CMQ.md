# Revision — run 01M2VH6A91ESRYJ0GGWQJ24CMQ

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2VH6A91ESRYJ0GGWQJ24CMQ.md
- seeds filed: none — ADR-0022 balance: 0 credit this pass, both findings journaled as overflow
- balance: 0 non-exempt seeds filed / 0 — no credit this pass
- basis: run 01M2VH6A91ESRYJ0GGWQJ24CMQ, workflow version 58a8171073f77ad9a1b6be42ffc1c46e128578a9b9875aa1a3fd2e4b3aa28c66, commit f317ac2566bcf8c074af7a4e5537cbfa06349ed2
- revised_at_commit: f317ac2566bcf8c074af7a4e5537cbfa06349ed2 (ADR-0015: engine drift signal for later judgement)

## Findings

### 1. Closeout deploy-gate note for engine-path seed work (server-image redeploy)
- filed: none — overflow-to-journal (0 filing credit this pass)
- concrete change: in `.fabro/workflows/develop/scripts/closeout.nu`, when the seed-work diff touches `lib/components/fabro-workflow/**`, append a 'deploy-gate: image redeploy required before this fix is observable in-run' line to the closure note/PR body and file a post-redeploy verification-only check.
- expected effect: next planner knows dogfooding is impossible pre-redeploy; no cycle burned verifying against a half-live engine.
- dedupe: `fabro-6f6e` (closed) covers only the Dockerfile/toolchain-image rebuild axis; sd searches for 'redeploy', 'image' return nothing covering the engine-path/server-image redeploy axis.

### 2. verify.nu: run `cargo test --no-run` on touched crates before fmt/clippy
- filed: none — overflow-to-journal (0 filing credit this pass)
- concrete change: in the implementer stage of `.fabro/workflows/develop/scripts/verify.nu`, run `cargo test --no-run -p <touched-crate>` ahead of fmt/clippy so signature-ripple compile misses surface at the cheapest phase.
- expected effect: compile errors stop paying the fmt+clippy toll first; faster, cheaper implementer verification on every Rust seed.
- dedupe: `fabro-2f70` is test-file detection, `fabro-a1ef` is rebuild-before-re-diagnosis, `fabro-9acb` is failure-display ordering; sd search 'no-run' returns nothing.
- basis detail: E0061 miss at two `preamble.rs` test call sites (~1780/1829) surfaced only at nextest compile after fmt/clippy inside the dominant 1102 s / $1.014 implementer stage of run 01M2VH6A91ESRYJ0GGWQJ24CMQ.
