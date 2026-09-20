# Improve review — run 01M2VAAYWA0H7XN0V8WSRQ29FE

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (21.6 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-18 23:18+0000 by revisor `fabro_ask`

---

# Recommendations — run 01M2VAAYWA0H7XN0V8WSRQ29FE (seed fabro-f84f, all 9 stages first-pass green, 21.2 min wall, $1.075 LLM: planner $0.329/193s, implementer $0.672/973s (639s tool), reviewer $0.074/53s, tester gate 29s warm, PR #246)

What already worked (no change): preflight in-flight exclusion (fabro-c643 skipped correctly), the planner's intermediate stale-basis correction via `sd update`, `claim_check`, and the warm-gate policy (tester green in 29s) — all from run events.

---

**1. The reviewer's evidence capture was elided despite being in budget — implement fabro-meta-c9f2.**
Evidence (run events, reviewer journal painpoint): the preamble's evidence section rendered "(170 lines omitted)"; the reviewer recovered the diff only by re-running `git diff 902f7a27` itself (5 shell calls, 53s stage). The capture was 11,209 bytes — *under* the reviewer's `preamble_inline_max_kb=16` and the 48 KB graph budget — yet the stage renderer still cut it. This is occurrence 5 of the class fabro-meta-c9f2 documents (119/144/115/60 lines previously; that seed is open, P1, and was literally in this run's preflight candidate table as claimable).
Change: `.fabro/workflows/develop/scripts/evidence.nu` + the engine stage renderer per fabro-meta-c9f2's fix directions — diff-first capture sized to the *rendered* window, and a marked blob ref whenever elision happens.
Expected effect: reviewer verifies from the capture instead of shell re-derivation; removes ~50s + 5 tool calls per review and the Verification-blocked escalation risk on larger diffs.

**2. The implementer ran the full test suite twice — implement fabro-2b1b.**
Evidence (run events, implementation_summary): the implementer ran a manual `cargo nextest run -p fabro-workflow` (1543 tests) *and* `just verify implementer`, which re-runs exactly that suite for test-file-touched crates (fabro-workflow was test-file-touched), plus hand fmt/clippy that verify also re-runs — all inside 639s of tool time. fabro-2b1b (open) documents the fmt/clippy arm of this duplication; this run shows the nextest arm.
Change: `.fabro/workflows/develop/prompts/implementer.md` step 4 and `prompts/planner.md` step 7 — briefs prescribe `just verify implementer` plus only genuine supplements (focused tests during development, feature-scoped clippy, cross-crate caller tests).
Expected effect: one fewer full suite/compile pass per test-touching implementer stage — tens of seconds here, minutes on multi-crate seeds.

**3. Guard the re-enabled slash-expansion exposure on the six opted-in stages.**
Evidence (run events, implementer + reviewer journals): both flagged that opting develop/architect/conductor stages into `skills="discover"` re-enables pebble expansion of bare `/name` tokens in harness-assembled input — the exact fabro-4dd8 killer that took the conductor line down in 0.3s (run 01M2GJXV71TD).
Change: add a deterministic lint to the loop-asset tier (alongside `lint-nu` in `scripts/qualitygate.nu` / validate-workflows) scanning the prompts and known context-rendered text of opted-in nodes for bare slash-tokens.
Expected effect: catches the session-death class before the next conductor pass fires, instead of via a dead line.
Seed: fabro-26c3 (open) is the durable upstream pebble fix; new-seed justification for the lint: no existing seed covers a fabro-side bare-slash guard — fabro-26c3 fixes pebble semantics, closed fabro-4dd8 was the incident, and nothing protects the interim.

**4. Enforce the reviewer's read-only posture — implement fabro-269d (user GO already given 2026-09-17).**
Evidence (run events, reviewer@1 stage data): the reviewer's tool registry again included `edit_file`, `write_file`, `fabro_run_interact`, `fabro_run_create`; it happened to invoke only 5 read-only shell calls.
Change: one attribute on the reviewer node in `.fabro/workflows/develop/workflow.fabro`: `tools="read_file,grep,glob,shell"`.
Expected effect: read-only becomes mechanical capability, shrinking the ADR-0019 surface to what the diff adds.

**5. Fix the preflight's anchor-path truncation.**
Evidence (run events, preflight output): fabro-c643 was flagged `anchors_ok=false` with path `.fabro/Dockerfile.toolchai` — one character short; the seed body (from workspace file `.seeds/issues.jsonl`) correctly says `.fabro/Dockerfile.toolchain`. Harmless here (c643 was skipped as in-flight), but planners adjudicating flag rows get corrupted data.
Change: fix the anchor-path extraction in `.fabro/workflows/develop/scripts/planner-preflight.nu`.
New-seed justification: fabro-7611 covers base-dir resolution and fabro-7daf content checks; no open seed covers extraction-time path truncation.

**6. Tell agents vendored crate sources aren't in the sandbox.**
Evidence (run events, planner transcript): the planner burned three calls probing `~/.cargo/git/checkouts` and a 1.6s `find /` for the vendored pebble crate (exit 1, empty) while re-deriving the stale diagnosis.
Change: one bullet in the PROJECT_FACTS block (`.fabro/workflows/develop/prompts/` include): judge engine/dependency behavior from the workspace tree only; cargo checkouts are absent in run containers.
Expected effect: removes the dead-end probe rounds on every engine-behavior seed (~10–15s and 2–3 calls each).
New-seed justification: no existing seed covers sandbox crate-source guidance; it's a PROJECT_FACTS fact, not a mechanism any filed seed owns.

Not inspected: PR #246 merge state and post-run journal consumption by the improve workflow (outside this run's events).
