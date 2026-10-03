# Revision — run 01M2AQY97EG1YDTSQES8FFFAS2

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2AQY97EG1YDTSQES8FFFAS2.md
- seeds filed: fabro-e137 (planner brief: `cargo nextest show-config` for config-only seeds, drop fmt/clippy bullets from non-Rust diffs), fabro-929f (write_locks: reject same-path concurrent writes within one batch), fabro-95c3 (project-doc discovery: log missing candidate docs at debug, not error)
- basis: run 01M2AQY97EG1YDTSQES8FFFAS2, workflow version e825d8c827ce15defe0aa57d3586b0f369cab48cc7fe711d526776762f442aa1, commit 282e0d22c820f419232f4e861a13033c4b2d8c94
- revised_at_commit: 282e0d22c820f419232f4e861a13033c4b2d8c94 (ADR-0015: engine drift signal for later judgement)

## Findings

### Planner brief: prescribe cargo nextest show-config (not list) for config-only seeds and drop fmt/clippy bullets from non-Rust diffs
- filed: fabro-e137
- Change: in `.fabro/workflows/develop/prompts/planner.md` step 6, name `cargo nextest list` as build-level and make `cargo nextest show-config` the parse-level check for nextest/config-only seeds; drop fmt/clippy bullets when the diff contains no Rust file. Expected effect: config-only runs fall from ~16 min to ~6-8 min (evidence: implementer tool time 655.8s of 848.7s wall vs 4.8s tester gate; first `cargo nextest list` timed out at 300s cold).

### write_locks: reject same-path concurrent writes within one batch instead of silently serializing
- filed: fabro-929f
- Change: in `lib/components/fabro-agent/src/write_locks.rs`, return a correctable tool error for two same-path writes in one parallel batch instead of silent serialization (keep the WARN). Cross-references open fabro-4601 (prompt-side half, complementary mechanism — not superseded). Expected effect: eliminates the silent-lost-hunk failure mode seen in the 19-warning predecessor run.

### Project-doc discovery: log missing candidate docs at debug, not error
- filed: fabro-95c3
- Change: in `lib/components/fabro-agent/src/memory.rs` `load_memory_documents`, classify missing optional candidate docs (e.g. `.codex/instructions.md`) and log at debug instead of ERROR from the fs driver. Expected effect: ~6 fewer false error lines per 3-agent run; error channel carries only real fs failures.
