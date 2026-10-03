# Improve review — run 01M2NZWP18E93VBR4H2QVGJMEH

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (2.3 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-16 20:58+0000 by revisor `fabro_ask`

---

## What actually happened in this run (grounding)

From run events and the run projection: a clean single-cycle pass — planner claimed `fabro-39ce`, implementer inserted 2 lines into `.fabro/workflows/develop/prompts/planner.md` (one 72 ms shell edit), tester green in 4.2 s, evidence 0.28 s, reviewer approved with **zero tool calls** in 14.1 s, closeout closed the seed. Wall 2 m 16 s, active 105.9 s, cost **$0.184** — of which planner $0.081 (44%), implementer $0.074 (40%), reviewer $0.030 (16%). No retries, no gate reds, no cycles. So the levers below are **latency and token overhead, not control-flow fixes** — the graph's deterministic guards (gatebounce, deadlock exits) correctly never fired.

## Recommendations, ordered by expected impact

**1. Batch planner reconnaissance into one shell call.** — Seed: **fabro-55a7** (open; complements fabro-c3b4).
Change: `.fabro/workflows/develop/prompts/planner.md` steps 1–4 — require `sd ready` (top-N view), `sd show <candidate>`, and the `git log --grep <seed-id>` basis probe in ONE chained shell call, as the planner already did once at seq 43.
Grounding: the planner spent 6 LLM rounds / 29.7 s inference / $0.081 (44 % of run cost) just to claim a seed whose choice was deterministic (top of `sd ready` by priority). `sd ready --limit 200` returned 200 seeds / 28.4 KB with `stdout_truncated: true` (seq 30); `fabro_runs_list` returned 32 runs, also truncated (seq 39). Three of six rounds were pure recon separated by ~5 s TTFT each.
Effect: 2–3 fewer LLM rounds per develop pass ≈ ~10–15 s wall and roughly 20 % of per-run cost, paid on **every** run of this line.

**2. Split the PROJECT_FACTS sd-command table per role.** — Seed: **fabro-52b4** (open; complements fabro-7b2a for the implementer's step-4 cap).
Change: the PROJECT_FACTS include in `.fabro/workflows/develop/prompts/reviewer.md` (and implementer) — reviewer gets no sd table at all; implementer keeps only `sd show`.
Grounding: from the stage prompts in the projection, the reviewer — which is read-only by capability and made 0 tool calls — carried the full 6-row tracker table including claim/close write forms it is forbidden to use; the implementer consumed 24.6 k input tokens to make a 2-line edit, much of it step-4 verification policy irrelevant to a config-only seed.
Effect: ~15–25 % input-token cut per agent stage per run; directly reduces the $0.18 baseline and the per-round TTFT payload.

**3. Gate skips the Rust arm on loop-asset-only diffs.** — Seed: **fabro-574d** (open).
Change: `scripts/qualitygate.nu` touched-crate derivation — when the per-seed diff touches no `lib/` crate, skip `cargo fmt --check --all`/clippy (keep the lint-nu loop-asset arm).
Grounding: tester output (stage `tester@1`) printed `no crates touched` and then still ran `== cargo fmt --check --all == format clean` — a workspace-wide Rust check on a prompt-text-only diff, against a 20 m node timeout sized for cold Rust builds.
Effect: 4 s saved here; minutes saved on the cold-cache worst case; removes a whole class of spurious gate-red surface for revision-only seeds.

**4. Implementer: fold recon and verification rounds.** — New-seed justification: fabro-55a7 is planner-scoped; no open seed covers implementer-side *read/verify* batching (fabro-4601 = edit serialization, fabro-dd4e = don't chain write+exec — different axes).
Change: `.fabro/workflows/develop/prompts/implementer.md` step 1 — chain the target-file read (`grep -n` + `sed -n`) into the dup-run-check preflight call, and chain `git diff --stat` + `just verify implementer` into one post-edit call.
Grounding: implementer = 7 shell calls across 6 LLM rounds, 50.6 s inference vs 1.0 s tool time; rounds at seq 78 and 84 were pure reads split across turns, each costing a ~5–8 s model round-trip (seq 88→89 shows an 8 s round just planning the insertion).
Effect: ~2 fewer rounds per implementer pass ≈ 8–12 s wall and ~$0.02; largest relative gain on small config/prompt seeds like this one.

**5. Bound the in-flight check and expose `current_seed_id` engine-side.** — Seeds: **fabro-6b58** and **fabro-9372** (both open; incident class tracked in fabro-a01f).
Change: planner node's `fabro_runs_list` usage (node attr + prompt step 4) — server-side default `created_since` window, self-exclusion, and `current_seed_id` in the projection so the planner stops reverse-engineering seed ids from journals.
Grounding: this run's call (seq 38) returned 32 runs over a 7-day window, output truncated, with this run itself listed — self-exclusion was model-inferred, not mechanical. The dup-run-check preflight itself worked (clean verdict, 618 ms, seq 75–76), but fabro-a01f documents the claim race recurring same-day (PR #197 vs #195).
Effect: one fewer large truncated parse per pass and a mechanical — not model-judged — self/seed match, closing the residual window the prompt-side fallback admits it cannot close.

**6. Batch checkpoint pushes at terminal boundaries.** — Seed: **fabro-652d** (open).
Change: engine checkpoint policy (see graph comment in `workflow.fabro`) — push at terminal/soft-exit boundaries instead of every stage boundary.
Grounding: this run produced 6 stage-boundary commits + pushes (`75e5ce8` → `42a9fa1` → `84ed2b0` → `f6cb63a` → `a9dfff2` → `55edf96`) to deliver a 2-line seed diff.
Effect: fewer GitHub round-trips and less event-log churn per run; modest wall-time, meaningful at the conductor's ~30 min cadence.

**Not recommended from this run's evidence:** changes to the reviewer's evidence pipe (capture was 5.7 KB, inline under the 16 KB `preamble_inline_max_kb`, zero blob detours, first-pass approval from context — exactly as designed), and PR-skipping (fabro-9f97) — this diff was not journal-only. Error handling was never exercised beyond the 618 ms clean preflight; the only live error-class demand is the engine-side admission re-verify already captured in fabro-a01f.
