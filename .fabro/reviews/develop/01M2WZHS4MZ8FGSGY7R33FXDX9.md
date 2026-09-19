# Improve review — run 01M2WZHS4MZ8FGSGY7R33FXDX9

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (33.5 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-19 15:03+0000 by revisor `fabro_ask`

---

All findings below are from this run's event history (stage outputs, journal painpoints, timings, per-stage cost) and verified against workspace files. Run shape for context: 32.8 min wall, $3.72 total, implementer@1 alone was 26.5 min / $3.35 (90% of cost, 156 tool calls); the seed (fabro-44d8, sandbox GC, +1,184/−5 across 15 files) closed after two review cycles.

## Recommendations, by expected impact

**1. The gate shipped this seed with its entire test suite never executed — fix the workspace-mode fallback in `scripts/qualitygate.nu`.**
What happened (from run events): the seed added `bollard` to root `Cargo.toml`/`Cargo.lock`, so both tester runs printed `root manifest/lock changed -> workspace-wide gate (cargo check)` and finished with lint-nu + fmt + `cargo check --workspace` — **no nextest line, "GATE GREEN"**. The implementer deliberately compile-verified only ("tests WRITTEN but NOT EXECUTED … test execution belongs to the deterministic tester step", implementer@1 journal), i.e. the whole safety net was the tester — and the tester's fallback is a compile check only (confirmed at `scripts/qualitygate.nu:203`–207; the nextest tier at :174–175 was skipped). Result: the 523-line `fork_seam_test.rs` anchor suite (12 tests, acceptance criterion 3) was never run before approval, and `auto_merge=true` on PR #280 squashes it in unexecuted.
Change: in `scripts/qualitygate.nu`, the root-manifest branch keeps `cargo check --workspace` but must still run the touched-crate nextest tier first — a workspace dep bump must not disable test execution.
Expected effect: latent test failures surface in-run instead of after auto-merge.
Seed: **new seed needed** — no open seed covers gate test execution in workspace mode (fabro-2f70 is verify.nu test *detection*; fabro-4815 is gate exit-status classification).

**2. The only review bounce was a rule the implementer was never told — teach and mechanize the fork two-layer rule.**
What happened (from run events): reviewer@1 ($0.096, 63.7s) rejected green work for exactly one deviation: the missing `touchpoints.md` row (two-layer rule, 00ffd60f6). That rule exists only in `prompts/reviewer.md`; `prompts/implementer.md` never mentions it — the implementer even coined mx-d04457 ("every fork feature anchored in fork_seam_test.rs") while shipping layer 1 only. The bounce cost a full re-cycle (planner@2 → claim_check → implementer@2 → tester@2 → evidence@2 → reviewer@2, ~2.6 min / ~$0.14) and consumed one of three review cycles before deadlock. It also forced the reviewer to grep `.agents/` (an fs_hide path it's told is unreadable — its journal painpoint).
Change: add the two-layer checklist bullet to `.fabro/workflows/develop/prompts/implementer.md`, and per the fabro-9ec3 standing policy make it mechanical in `scripts/verify.nu`: diff adds fork-owned engine files → require a matching `.agents/skills/merge-upstream/references/touchpoints.md` row before PASS.
Expected effect: fork-feature seeds ship both layers first pass; this bounce class disappears and the reviewer no longer greps fs_hide paths to verify layer 2.
Seed: **fabro-b659** (known) covers the complementary mitigation — a reviewer→implementer "minor changes" edge so a one-row fix skips the full planner re-plan; the prevention side above needs a new seed (no open seed teaches the implementer this rule).

**3. The reviewer lost the middle of a 58 KB evidence blob — implement the known blob-formatting seeds.**
What happened (from run events): the capture was 57,965 bytes (cycle 1) against the 48 KB preamble budget, so it blob-ref'd as single-line JSON; reviewer@1's journal: "~10 KB of its middle — the entire head of `sandbox_gc.rs` — elided in both read_file and grep renders; I had to recover it by reading the repo file directly since the worktree was clean." That fallback only works on a clean tree (cycle 2's capture grew to 77,427 bytes).
Change: `evidence.nu` emits the capture as plain-text multi-line instead of JSON-escaped single-line, and raise the reviewer node's `preamble_inline_max_kb` 16→32 in `workflow.fabro`.
Expected effect: reviewer pages the full capture; no dependence on a clean worktree, no risk of misread or wasted "Verification blocked" cycles on large diffs.
Seed: **fabro-8cef** (plain-text multi-line blob) and **fabro-cf3e** (16→32 inline budget) — both already open and exactly on point.

**4. Every Rust run re-hits the missing `libssl-dev` — bake it into the toolchain image.**
What happened (from run events + workspace): implementer@1 burned ~45s diagnosing an `openssl-sys 0.9.116` build dead-end (native-tls via the pinned daytona-sdk) before `apt-get install -y libssl-dev`; the install dies with the sandbox, so the next run re-hits it. Root cause confirmed in the workspace: `.fabro/Dockerfile.toolchain:45` installs `build-essential pkg-config perl cmake` but **not** `libssl-dev` (`.fabro/Dockerfile:4`, the non-toolchain image, does).
Change: add `libssl-dev` to the apt line in `.fabro/Dockerfile.toolchain` (or unify reqwest to rustls-only so native-tls leaves the graph).
Expected effect: removes a guaranteed dead-end diagnosis from every Rust-touching run.
Seed: **new seed needed** — no open seed covers the toolchain image's missing libssl-dev.

**5. The planner drank two firehoses and hand-recovered a seed id — implement the two known projection/economy seeds.**
What happened (from run events): `sd ready --assignee fabro --limit 200` returned all 200 seeds (28,750 bytes of stdout into planner context, event seq 46), `fabro_runs_list` returned 85 runs, and because the projection carries no seed id, the planner manually grepped `.fabro/journal/01M2WECF*.jsonl` to confirm run 01M2WECF hadn't claimed fabro-44d8 (seq 53–55) — exactly the reverse-engineering these seeds exist to stop.
Change: top-N `sd ready` view for the planner (fabro-c3b4) and expose `current_seed_id` in the `fabro_runs_list` projection (fabro-9372, engine-side, `lib/components/fabro-workflow`).
Expected effect: ~7k tokens less planner context per claim, one less shell round-trip and no journal-grep fallback.
Seed: **fabro-c3b4** and **fabro-9372** (both known, both open).

Not worth acting on from this run: tester cold time (81.8s, and warm 12.5s on re-run) is healthy; reviewer/planner costs ($0.13/$0.19) are already at the low-effort steady state; the single `context_allow_keys dropped: output.planner` warning is engine-internal dedup noise with no observed downstream effect.
