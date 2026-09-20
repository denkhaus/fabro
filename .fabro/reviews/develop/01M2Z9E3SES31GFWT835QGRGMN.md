# Improve review — run 01M2Z9E3SES31GFWT835QGRGMN

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (32.4 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-20 12:32+0000 by revisor `fabro_ask`

---

All recommendations below are grounded in this run (01M2Z9E3SES31GFWT835QGRGMN, seed fabro-94f6, one clean cycle: 0 retries, gate green, first-pass approval; wall 31.9 min, $1.54 total, implementer = $1.34 / 87% of cost — from run events). For calibration: the loop's structure worked; everything below is friction that actually fired.

---

**1. The gate and `just verify` never ran this seed's regression pins — the `it` suite is silently skipped (correctness, not cost).**
Evidence: `lib/apps/fabro-server/Cargo.toml:15-18` declares `[[test]] name = "it" required-features = ["test-support"]`, but `scripts/qualitygate.nu:190` runs `cargo nextest run -p <crate> --no-fail-fast --retries N` and `scripts/verify.nu:134` runs `cargo nextest run -p $c --no-fail-fast` — neither passes `--features test-support`, so cargo skips that target entirely. The tester's "GATE GREEN / tests green" therefore covered unit tests only; the two acceptance-criterion regression pins (`environment_resources_disk_is_rejected_for_docker_at_write_time`, `..._accepted_for_a_provider_that_enforces_it` in `tests/it/api/environments.rs`) ran only in the implementer's *manual* `--features test-support` invocation (implementer journal painpoint #2, run events).
**Change:** in both scripts, derive `required-features` from each touched crate's `[[test]]` declarations and pass the matching `--features` to nextest.
**Effect:** the deterministic gate actually executes the it-suite; a red regression pin can no longer ship through a green gate.
**Seed:** fabro-6e7f (in_progress, owns the verification dispatcher; the implementer's journal already tags this as its follow-up).

**2. The reviewer judged a Rust diff without being able to read the binding style guide.**
Evidence: reviewer journal painpoint — `.fabro/skills/rust-style-guide/SKILL.md` is unreadable by reviewer file tools and the reviewer has no shell (fabro-269d); its session listed `rust-style-guide` as *available* but `activated: []` because `use_skill` is not in the reviewer's `tools="read_file,grep,glob"` allow-list (run events, reviewer stage). The reviewer approved a Rust diff whose binding standards axis it could not open.
**Change:** in the `reviewer` node of `.fabro/workflows/develop/workflow.fabro`, add `use_skill` to the tools allow-list (discovery already works), and/or have `evidence.nu` materialize the brief's `guide:` pages into the reviewer sandbox next to the blob.
**Effect:** the Rust standards axis becomes mechanically loadable at review time; Rust reviews stop being guide-blind.
**Seed:** fabro-81b7 (open — "implementer (and reviewer) must load and validate against the rust-style-guide skill").

**3. The 22.9 KB evidence capture was demoted to a blob ref — the per-value inline cap (16 KB) is below real capture sizes.**
Evidence: evidence output was 22,941 bytes (run events); the reviewer prompt rendered `Output (23.1 KB; full value: /tmp/sandbox-driver/runtime/blobs/bd9bd084….json)` and the reviewer's single tool call was paging that blob — while its context window sat at 2.1% usage and the graph budget is 48 KB (fabro-1e9f). The reviewer node's `preamble_inline_max_kb=16` (set when captures were ~14 KB) is the binding constraint, not the budget.
**Change:** raise the reviewer node's `preamble_inline_max_kb` from 16 to 32 in `workflow.fabro` (fits the 48 KB graph budget).
**Effect:** per-seed captures render inline; zero blob round-trips per review; removes the unread-blob → Verification-blocked rejection class (fabro-meta-c9f2 history).
**New-seed justification:** no open seed raises the reviewer's per-value inline cap — fabro-1e9f (closed) raised only the graph-level budget, and this run proves the 16 KB per-value cap still demotes ~23 KB captures.

**4. The planner paid for two unbounded firehoses it didn't need.**
Evidence: `sd ready --assignee fabro --limit 200` returned 200 issues / 28.6 KB, `stdout_truncated: true` (planner event seq 46-47), and `fabro_runs_list` returned all 114 develop runs unbounded (seq 55, ~16 s for that call alone) — for a decision that needed the top candidate and an in-flight check. Planner conversation context: 51.7 k tokens.
**Change:** per the seeds — planner.md step 1: top-N view (`sd ready --first 10` / priority filter, full listing only as fallback); step 4: `created_since ≈ 48h` + explicit self-run exclusion.
**Effect:** ~40 KB → a few KB of tool output per planning pass; direct token/latency cut on every run.
**Seeds:** fabro-c3b4 and fabro-6b58 (both open, cover exactly these two arms).

**5. The implementer's in-lane test run tripped over 3 environment-artifact graph-render failures the gate already knows how to avoid.**
Evidence: implementer observation — `get_graph_returns_svg` / `render_graph_from_manifest_*` failed in its direct nextest run (render subprocess empty output in-sandbox), forcing a git-stash verification plus an extra full fabro-server suite run; the reviewer's observation flagged exactly this as "duplicated tester work on that crate". The gate avoids it via `build-renderer-if-needed` (`scripts/qualitygate.nu:206-216`); `scripts/verify.nu` has no equivalent before its nextest step (line ~132).
**Change:** `scripts/verify.nu` calls the same `cargo build -p fabro-cli --bin fabro` prebuild before nextest whenever fabro-server is in the touched set.
**Effect:** no phantom failures in the implementer lane, no stash-dance classification, no duplicated suite run (~3-4 min + tokens in this run).
**New-seed justification:** fabro-febd is closed on a skip-guard that did not prevent this run's in-lane failures, and its fix landed gate-side only — no open seed covers the verify.nu-side prebuild arm.

**6. Standalone crate builds still dead-end on openssl-sys.**
Evidence: implementer journal painpoint #1 — `cargo clippy -p fabro-sandbox` fails building openssl-sys 0.9.116 (reqwest default-tls via daytona-sdk); the workaround was exporting `OPENSSL_DIR` to the prebuilt vendored install already in the shared target dir — re-derived at cost in this run after the same dead-end in run 01M2WZHS.
**Change:** root fix per the existing seed — bake `libssl-dev` into `.fabro/Dockerfile.toolchain` (needs-user, ADR-0019); no-rebuild stopgap: `scripts/verify.nu` auto-sets `OPENSSL_DIR` when a prebuilt vendored install exists under `CARGO_TARGET_DIR` (the implementer's own fix idea).
**Effect:** removes a recurring ~45 s+ dead-end diagnosis from every run that builds fabro-sandbox (or any crate pulling reqwest default-tls) standalone.
**Seed:** fabro-95e1 (open, needs-user — covers the root cause; the verify.nu stopgap is its in-run arm).

**7. Every agent stage still carries full-tracker command tables and the full AGENTS.md memory regardless of role.**
Evidence: AGENTS.md (25,746 bytes) loaded into all three agent sessions — memory tokens: planner 7,744 / implementer 6,122 / reviewer 6,371 (context-window breakdowns, run events); the read-only reviewer's prompt advertises the complete sd claim/close command table it is forbidden to use (reviewer prompt, run projection).
**Change:** per the seeds — split the PROJECT_FACTS sd table per role in `.fabro/workflows/develop/prompts/` (reviewer: none; implementer: `sd show` only), and scope memory/skills per agent node.
**Effect:** ~6-7 k tokens cut per stage and forbidden-command surface removed from read-only roles (small $, pure hygiene).
**Seeds:** fabro-52b4 and fabro-9588 (both open).

---

What I could not inspect: the tool transcripts' shell-level detail for the implementer's stash dance (only its journal report and reviewer corroboration), and whether `pull_request.state: null` on PR #322 (prior run) ever misled a planner — no friction from it appeared in this run.
