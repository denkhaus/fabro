# Improve review — run 01M23RQRCB748R4VRXA2WE6BZ0

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (17.7 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-09 19:24+0000 by revisor `fabro_ask`

---

All findings below are grounded in this run's events, checkpoints, journals, stage timings, billing, and worker log (run `01M23RQRCB748R4VRXA2WE6BZ0`, 19:01–19:19, 1,045 s wall, $0.72 LLM; PR #109). Cost context: the **failed** implementer pass alone burned 731 s / $0.373 — **52% of run cost** — on work that duplicated merged PR #107; the two planner passes added another 195 s / $0.256.

## 1. Make the implementer detect an already-merged seed before editing (highest impact: −12 min, −$0.37, no human halt)
- **What happened:** Planner claimed `fabro-6a5a` at 19:02:56; the in-flight check (run events seq 37–47) correctly saw run 01M23Q0 as terminal with PR #107 *merged* — not "in flight" — while the tracker still showed the seed open (planner@2 journal: "tracker left fabro-6a5a in_progress after #107 merged"). Implementer@1 then worked 11.5 minutes (692 s wall, 252 s inference + 438 s tools) re-implementing #107 until a human line-watch operator halted it ("duplicate run: fabro-6a5a already merged as #107", worker log ERROR 19:14:47).
- **Change:** `.fabro/workflows/develop/prompts/implementer.md`, step 1 ("Re-read the seed requirements from `sd show`") — add: before any edit, run `git fetch origin <base> && git log --oneline origin/<base> --grep "<current_seed_id>"`; if the seed id appears in a merged commit, route `Blocked` with `duplicate run: <seed> already merged as #<n>` and change nothing. This is exactly what the human halt did, automated.
- **Expected effect:** this failure class costs one shell call (~1 s) instead of 12 minutes, $0.37, and a manual intervention.

## 2. Stop checkpointing a deterministically-failed stage's diff onto the run branch — it shipped user-reverted changes in PR #109
- **What happened:** Implementer@1's duplicate diff (auto_merge=false in 3× `workflow.toml`, `pull_request.rs` warn→info + test) was committed as `ae95484` despite the failed outcome. Planner@2 checked `git status`, saw clean, and journaled "the uncommitted duplicate diff is gone… no cleanup needed" — it was in *history*, not gone. The final run diff (7 files, +85/−13, from run conclusion) still contains it, so **PR #109 re-introduces `auto_merge = false` — the exact half of #107 the user reverted** ("auto-merge must stay", implementer@1 journal).
- **Change:** Engine checkpoint path (the pipeline code that produced commit `ae95484`, `lib/components/fabro-workflow/src/pipeline/`): roll back the worktree (or commit to a side ref) when a stage fails with `failure_class=deterministic`. Interim prompt fix: `.fabro/workflows/develop/prompts/planner.md` re-plan step — after a Blocked implementer, audit `git diff <run-base>..HEAD --stat` (never `git status`) and supersede/revert leftovers before claiming the next seed.
- **Expected effect:** PR #109 would have carried the 2-line `implementer.md` change + bookkeeping, not 85 lines including a setting the user explicitly reversed.

## 3. Fix `evidence.nu` claim-base resolution and loop-work diff gating (the reviewer's own painpoint)
- **What happened:** The capture (evidence stage output) listed `pull_request.rs +58/-3` as seed work **with an empty diff body**, filed the real seed work (`implementer.md +2/-0`) under loop churn with no loop-work diff shown — reviewer journal: "the capture delivered neither scope correctly… the capture appears to have diffed against a base that predates the claim base, resurrecting the failed attempt's hunks." The reviewer had to re-derive scope with its own `git diff b8e20c9..HEAD`. Same stale base fed the gate: tester output "touched crates: fabro-workflow" for a **Markdown-only** seed → full crate fmt+clippy+nextest (23.5 s).
- **Change:** `.fabro/workflows/develop/scripts/evidence.nu` — (a) resolve the per-seed base as the commit where `current_seed_id` changed, excluding failed-stage commits; (b) make "listed seed-work file with empty diff body" a hard error; (c) render the loop-work diff whenever changed files intersect the seed's named targets, not only when seed-work count = 0. Apply the same base fix to touched-crate detection in `scripts/qualitygate.nu`.
- **Expected effect:** the reviewer judges from the capture instead of distrust-and-rederive; prompt-only seeds stop paying a Rust crate gate.

## 4. Scope the review to the full PR surface, not just the per-seed base
- **What happened:** The reviewer verified `git diff b8e20c9..HEAD` ("no other prompts, no workflow.toml, auto_merge untouched") and **Approved** — while the actual PR #109 (vs run base `ae8c181`) contained the workflow.toml trio + `pull_request.rs`. The claim-base framing hid the duplicates below it.
- **Change:** `.fabro/workflows/develop/prompts/reviewer.md`, "Your job this pass" step 2 — add: "Account for every file in `git diff <run-base>..HEAD --stat` (run base from the capture header); changes below the claim base from earlier failed stages are deviations unless the seed names them."
- **Expected effect:** duplicate/user-reverted changes can no longer ride an approval.

## 5. Prioritize the prevention seeds over prompt-polish seeds (the race was a known recurrence)
- **What happened:** Planner@2's painpoint opens "Duplicate-run claim race **recurred**… matches seed fabro-6b58's direction"; the planner prompt itself cites open engine seed fabro-9372 (`current_seed_id` in the runs projection). Both sat open while this run's two claims (fabro-6a5a, fabro-e702) were config/prompt polish. fabro-9372 alone would have made "seed X already merged via #107" a one-line skip.
- **Change:** Tracker priority/assignment (user-side), or a tie-break line in `prompts/planner.md` step 2: prefer seeds that close a failure class observed in recent runs over same-priority polish seeds.
- **Expected effect:** the 12-minute duplicate class dies engine-side instead of being journaled a third time.

## 6. Cut planner latency: ~97 s and $0.13 per pass, ×2 = 36% of run cost
- **What happened:** planner@1: 92.0 s inference vs 4.8 s tools across 8 separate tool calls (seq 31–89); planner@2 nearly identical (93.5 s / 4.2 s). The in-flight check alone cost 3 round-trips (runs_list → journal grep → resolution) because goals carry no seed id.
- **Change:** `prompts/planner.md` — batch reconnaissance into one shell call (`sd ready` + `sd show` + base-branch grep in a single command), and land fabro-9372 so the journal fallback disappears entirely.
- **Expected effect:** ~40–60 s and ~$0.06 saved per planner pass, and a shorter claim→dispatch window (21 s here: claim 19:02:56 → implementer start 19:03:17), which directly narrows the race window from #1.

## 7. Smaller, cheap fixes
- **Enforce one edit per file per tool batch:** worker log shows two `concurrent write to the same file in one batch; serializing path="…/pull_request.rs"` warnings (19:05:15, 19:05:31) from implementer@1 — open seed fabro-1d94's exact one-line fix for `prompts/implementer.md`. Effect: removes the silent-ordering hazard at zero cost.
- **Downgrade the by-design absent-key warn:** `preamble_allow_keys entry absent … key=output.gate_known_bug_hits` fired on both implementer visits (19:03:17, 19:16:36) though that key only exists after a gate-red bounce — open seed fabro-8275's info-level downgrade. Effect: the warn channel stays meaningful for real errors like the 19:14:47 halt.

Not inspected: PR #109's post-merge state on GitHub (engine-mediated only; I could not verify whether the duplicate `auto_merge=false` ultimately landed or was reverted again) — recommendation #2/#4 stand on the run-branch diff regardless.
