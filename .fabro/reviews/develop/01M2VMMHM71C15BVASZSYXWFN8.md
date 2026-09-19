# Improve review — run 01M2VMMHM71C15BVASZSYXWFN8

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (7.7 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-19 01:43+0000 by revisor `fabro_ask`

---

All facts below are from this run's events/checkpoints (`fabro_run_get`, run events seq 1–178) and from workspace files under `/workspace/fabro/.fabro/workflows/develop/`. Run shape for grounding: verification-only pass on **fabro-6a78**; wall 443 s active, cost **$0.268**, of which planner = **$0.220 / 82 %** (95.8 s inference, 14 tool calls) and reviewer = **$0.048** (31 s inference, **287 s tool time**). No retries, no gate cycles, zero journal painpoints.

## Recommendations, ordered by expected impact

**1. Build the warm Rust toolchain image — seed `fabro-c643` (open, High, user GO 2026-09-17 already recorded in its body).**
What happened here: the reviewer's single targeted check (`cargo nextest run -p fabro-types -p fabro-workflow context_keys preamble`, journal: "100/100 green") consumed the bulk of its **287 s tool time** on a cold container (`lifecycle.preserve=false`, image `99c855a689b3`); the seed's own basis measured 15-min cold gates. Change: one `.fabro/Dockerfile.toolchain` rebuild with cargo-chef layers (the four consolidated arms). Expected effect: 4–7 min off every Rust-verifying run's wall — the largest lever visible in this run, and it was the top-priority seed skipped only because it's in-flight elsewhere.

**2. Cap the planner's tracker firehose — seed `fabro-c3b4` (open; companions `fabro-e4fa`, `fabro-55a7`).**
What happened here: the planner's first call `sd ready --assignee fabro --limit 200` poured **29,133 bytes / 200 seed rows** into the conversation (event seq 38), and `fabro_runs_list` returned 69 runs over a 9.3 s call (seq 40–41) — feeding a 53k-input-token planner stage that is 82 % of run cost. Change: `prompts/planner.md` step 1 — top-N view (`sd ready --first 10` / priority filter) plus `created_since` on `fabro_runs_list`, batching recon per `fabro-55a7`. Expected effect: fewer planner turns and tokens on the dominant cost stage; shorter claim window also narrows the duplicate-claim race.

**3. Port the rg/glob footgun one-liners to the planner prompt — seed `fabro-6997` (open).**
What happened here: the planner ran `rg -rn "starts_with(\"current"` (seq 51), `-r` silently replaced matches with literal `n` (`pub const n: &str = "current."`), and it took ~3 wasted probes plus a full LLM round to self-diagnose ("My earlier `rg -rn` mangled output", seq 86) — on the exact trap `implementer.md` already warns about but `planner.md` doesn't. Change: add the two one-liners (never `rg -rn`; glob on fs_hide paths returns empty) to `.fabro/workflows/develop/prompts/planner.md`. Expected effect: −2–3 tool calls and −1 LLM round per planner pass touching platform paths.

**4. Emit per-criterion check outputs in verification-only captures — seed `fabro-f759` (open).**
What happened here: `evidence@1` produced a **2,577-byte** capture that was mostly "(no seed-work files to diff)" and the seed spec, so the reviewer re-derived all five criteria itself with 4 shell calls and a cold test run (287 s tools). Change: `.fabro/workflows/develop/scripts/evidence.nu` — when seed-work is empty and the brief is verification-only, run the brief's cheap check commands and append outputs as a capture section. Expected effect: reviewer approves from the capture (3→0 shell calls), removing duplicated verification and implementer/reviewer drift.

**5. Fix the dead gate criterion in the Verification-only brief template — NEW SEED.**
*Justification: I grepped the tracker; `fabro-f759`/`fabro-b8ed` cover evidence capture and satisfaction assertions, and no open seed names the planner template's canned gate line for the tester-skipping route.* What happened here: brief criterion 5 said "gate green via the deterministic tester step", but the `planner -> evidence` Verification-only edge (workflow.fabro, fabro-9d26) skips the tester by design — the criterion is structurally unsatisfiable, and the reviewer improvised its own nextest scope to compensate. Change: in `prompts/planner.md`'s Verification-only JSON template, replace the gate bullet with "targeted tests green via named nextest filter (timeout_ms >= 600000)". Expected effect: verification-only briefs carry only checkable criteria; reviewer scope stops being improvised.

**6. Make closeout honor seed-body close-conditions — NEW SEED (extends the `fabro-534e`/`fabro-7aac`/`fabro-22fa` re-file family, none of which covers seed-body "Close when" arms).**
*Justification: those three seeds re-file EXEMPTION arms, implementation-summary deferrals, and reviewer journal findings respectively — no open seed covers external close-conditions written in the seed body itself.* What happened here: fabro-6a78's body says "Close when #786 merges upstream AND denkhaus carries the merge"; the reviewer journal explicitly flagged that the deterministic closeout would bypass it "by design" — and `closeout.nu:205` ran a bare `sd close $seed_id` (no reason, no re-file) 3 s later. The #786 tracking obligation now lives only in this run's journal. Change: `closeout.nu` — grep the seed body for "Close when" arms before closing; unmet external conditions get re-filed as a low-priority tracking seed (or recorded in the close reason). Expected effect: external obligations survive seed closure instead of dying silently.

**7. Backfill `pull_request.state` for terminal-but-unmerged runs — seed `fabro-4bb7` (open).**
What happened here: the planner's in-flight adjudication reasoned over five succeeded runs with `pull_request.state: null` (seq 41, reasoning seq 44: "null states on succeeded terminal runs... aren't mid-flight") — it concluded correctly, but only by judgment, and the seed's basis shows the same nulls forcing documented degraded mode elsewhere. Change: engine `fabro_runs_list` projection — populate PR state for terminal unmerged runs. Expected effect: the in-flight guard stops relying on LLM interpretation exactly when a PR sits unmerged.

**What already worked (no change):** the deterministic preflight verdict table (`output.preflight`) correctly excluded in-flight fabro-c643, the schema-validated brief contained no gate command, and both journals answered on the first pass — the friction above is cost/economy, not correctness of the loop's guards.
