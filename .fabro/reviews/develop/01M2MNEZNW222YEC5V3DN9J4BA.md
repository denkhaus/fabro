# Improve review — run 01M2MNEZNW222YEC5V3DN9J4BA

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (6.2 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-16 08:42+0000 by revisor `fabro_ask`

---

All evidence gathered. Recommendations below, ordered by expected impact. Every number is from this run's events (run `01M2MNEZNW222YEC5V3DN9J4BA`, seed `fabro-1e9f`, a +5/−1 config change that went planner→implementer→gate→evidence→reviewer→closeout first-pass in 359s wall / $0.231), and every seed id was verified still open in the tracker (workspace file `.seeds/issues.jsonl`).

**Context: what already worked (don't touch):** the config-only gate fast path (`just qualitygate` green in 3.6s, "no crates touched"), the churn-only evidence capture (272ms, 3.3KB, no blob ref, reviewer approved in 31.6s with one tool call), and the planner's stale-basis check — it caught that half the seed was already implemented and corrected the body before claiming, which is exactly why this run was cheap. The friction is in the surrounding machinery:

## Recommendations

**1. Pre-build the `fabro-validate` binary in the toolchain image — 119.5s of this run's 359s (33%) was one cold build.**
Evidence: implementer shell call `just validate-workflows develop` (event seq 133) ran 119,523ms to validate a 5-line graph edit; the warm re-run at seq 153 took 90ms. The justfile itself labels this the "test-harness cold build". This single call was over half the implementer's 236s wall.
Change: bake `fabro-validate` into `ghcr.io/denkhaus/fabro-toolchain` (or cache `target/` across stages), so `validate-workflows` never cold-builds inside a run.
Expected effect: ~2 minutes off every loop-asset seed's wall time; here that's 33% of the run.
Seed: **new seed needed** — `fabro-cfd6` covers prebuilding the *CI dogfood-gate* image and `fabro-80ec` covers the mise toolset, but neither covers the in-run `validate-workflows` cold build observed here.

**2. Batch checkpoint pushes at terminal boundaries — this run made 7 pushes, 3 in the final 8 seconds.**
Evidence: `git.push` at seq 69, 167, 175, 183, 209, 217, 220 — one per stage boundary, including back-to-back pushes for the 3.6s tester, 272ms evidence, and 377ms closeout command nodes (seq 217 at 08:37:25.1, seq 220 at 08:37:26.0).
Change: engine keeps per-stage commits, pushes only at terminal/soft-exit boundaries (seed `fabro-652d`, file: engine checkpoint pipeline, `lib/components/fabro-workflow/`).
Expected effect: ~10–20s wall off every develop run regardless of seed size, plus 5 fewer GitHub push round-trips per run.
Seed: **fabro-652d** (exact match, cites the same per-boundary overhead).

**3. Replace the `sd ready` firehose with a top-N view — 27.7KB of truncated seed list to pick line 1.**
Evidence: planner's first call (seq 30–31) returned 200 seeds / 27,701 bytes with `stdout_truncated: true`; the planner then claimed the first line (`fabro-1e9f`, High priority).
Change: planner prompt command table → `sd ready` with a priority-sorted top-N (fall back to full list only when top candidates are unclaimable), per `fabro-66bc` (also covers `fabro-c3b4`, same class).
Expected effect: ~4k tokens less planner context per pass, no truncation noise; planner was 37% of run cost ($0.086).
Seed: **fabro-66bc** (with **fabro-c3b4** as the duplicate to supersede on landing).

**4. Bound the in-flight check — 12.6KB of run projections to conclude "only I am running".**
Evidence: `fabro_runs_list` (seq 33) returned 14 runs / 12,610 bytes, 13 terminal with byte-identical goal text; the planner's reasoning had to deduce self-exclusion manually.
Change: pass `created_since ≈ 48h` and an explicit self-exclusion line in planner step 4 (`fabro-6b58`, file: `.fabro/workflows/develop/prompts/planner.md`).
Expected effect: ~10KB less tool output and removes the self-run confusion class; engine-side completion is `fabro-9372`.
Seed: **fabro-6b58**.

**5. Batch planner reconnaissance into one shell call — 6 sequential round-trips, 7 LLM turns, 15.6s first-token latency.**
Evidence: seq 29→55 = `sd ready`, `fabro_runs_list`, `sd show`, basis grep, `sed` inspection, `sd update`+claim, each costing a full model turn (TTFT alone was 15.6s at seq 27); planner inference 68.6s.
Change: one combined shell call for the read-only recon (`sd show <top> && git log --grep && grep basis anchors`), per `fabro-55a7` (file: `planner.md`).
Expected effect: 3–4 fewer LLM round-trips ≈ 30–45s off every planning pass; also narrows the claim-to-dispatch duplicate-claim window.
Seed: **fabro-55a7**.

**6. Skip the implementer's `sd show` re-read when the brief is complete — it re-fetched a seed the brief already quoted verbatim.**
Evidence: implementer call at seq 84–86 re-read `fabro-1e9f` (1,326 bytes) although `current_seed_brief` contained the entire corrected seed body word-for-word, including the CORRECTION paragraph.
Change: make step 1's re-read conditional on thin/ambiguous/verification-only briefs (`fabro-4881`, file: `.fabro/workflows/develop/prompts/implementer.md`).
Expected effect: one fewer call and one fewer model turn per cycle, and the planner's resolved reading stays authoritative.
Seed: **fabro-4881** (exact match — same behavior, prior run).

**7. Post-merge sibling sweep in closeout — two sibling seeds are now lying open in the tracker because of this run.**
Evidence: the planner journaled "next planner should superseded-close `fabro-8d2c`" (checkpoint seq 67) and the reviewer repeated it (seq 207), but nothing in-run can act on it — closeout only closes the current seed. I verified in the tracker that **both `fabro-8d2c` (raise to 32) and `fabro-35ab` (raise to 20) are still open** even though this run landed 48.
Change: extend `.fabro/workflows/develop/scripts/closeout.nu` with a grep of the run diff against open seeds targeting the same file/attribute, flagging satisfied siblings (`fabro-ab38`).
Expected effect: tracker stays truthful after every merge; directly prevents the next planner from claiming already-implemented work (the double-pick class) — in this tracker, two such seeds exist right now.
Seed: **fabro-ab38** (exact match).

**8. Document grep exit 1 as expected no-match — the run's only "error" was a benign one.**
Evidence: the implementer's single errored shell call (seq 139–140): exit 1, 41ms, zero output — consistent with the "no stray `preamble_budget_kb=24`" no-match grep it was running as verification; `agent.tools.shell.errors: 1` otherwise tars a clean pass.
Change: one line in the implementer prompt's verification section ("grep exit 1 = no match = expected state") per `fabro-7808`.
Expected effect: removes a recurring false-error signal that has historically produced "sandbox rg unreliable"-style misdiagnoses.
Seed: **fabro-7808** (exact match — same benign exit-1 signature).

**9. Split the PROJECT_FACTS sd-command table per role — forbidden commands shipped to roles that can't use them.**
Evidence: the implementer (24.0k input tokens) and reviewer (18.5k input tokens) prompts both carried the full planner claim/close/superseded-close command table, including forms both roles are explicitly forbidden to use, plus a 25,271-byte `AGENTS.md` memory load in all three sessions — heavy fixed cost for judging a 5-line diff.
Change: per-role include in `.fabro/workflows/develop/prompts/` — planner full table, implementer `sd show` only, reviewer none (`fabro-52b4`).
Expected effect: ~1–2KB less prompt per stage per run and the attack surface of forbidden tracker commands shrinks to the one role allowed to use them.
Seed: **fabro-52b4**.

## Priority logic

Items 1–2 are pure wall-time (≈33% + ~5% of this run) with no behavioral risk. Items 3–6 are the planner/implementer economy (the planner alone was 37% of cost and 20% of wall for a claim that a top-N list would have made near-instant). Item 7 is the correctness item — it's the only recommendation addressing an active defect this run *created* (two stale sibling seeds). Items 8–9 are cheap hygiene.

One caveat: `fabro-66bc`/`fabro-c3b4` and `fabro-55a7`/`fabro-6b58` overlap in target file (`planner.md`) — landing them as one combined planner-economy seed with all four ids cited would avoid a fourth revision pass over the same prompt.
