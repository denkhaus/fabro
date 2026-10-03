# Improve review — run 01M2NA1Z3HS7QPQR8AEWZR3GDB

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (3.5 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-16 14:42+0000 by revisor `fabro_ask`

---

All evidence is gathered (full planner/reviewer transcripts, timings, costs, journal, final diff, and the tracker at `.seeds/issues.jsonl`). Run 01M2NA1Z3HS7QPQR8AEWZR3GDB was a clean verification-only pass: planner claimed fabro-6a77 (criteria already landed under fabro-67e5 via upstream merge 0cac5a8), evidence captured, reviewer approved via 2 rg batches, closeout closed the seed, PR #187 opened. Zero retries, zero gate cycles. Total: 3m05s wall, $0.219, of which the planner was 149s / $0.185 (85% of cost, 81% of wall). Everything below is grounded in that.

## Recommendations, ordered by expected impact

**1. Batch the planner's reconnaissance probes — 15 sequential shell calls drove 143s of inference.**
From run events: the planner made 16 LLM turns / 15 shell + 1 `fabro_runs_list` calls; seqs 43–111 are ~10 single-purpose probes to locate the publish machinery (`ls .fabro/workflows/develop/`, grep `publish_failed`, grep `publish` in workflow.fabro, `rg publish_failed lib/`, `rg` in finalize.rs, `sed -n` finalize, `rg PublishBlocked`, `rg "json|parse"`, `rg readiness`, `sed -n` initialize.rs, `rg pr-scope`), each costing a 6–10s LLM round-trip. All of it answered one question: where does publish/PR code live.
**Change:** `.fabro/workflows/develop/prompts/planner.md` — make stale-basis probing one labeled compound shell call (all greps + `git log --grep` in a single invocation) instead of N turns.
**Seed:** fabro-55a7 (open, "Planner: batch reconnaissance into one shell call").
**Effect:** ~8 fewer LLM turns ≈ 60–80s wall and roughly $0.05–0.08 off every develop pass; on this run it would have halved planner time.

**2. Enforce the claim-time stale-spec correction deterministically — the planner skipped a mandated step.**
From run events: the planner correctly diagnosed fabro-6a77's wrong path ("publish step of .fabro/workflows/develop/workflow.fabro" — no such node), journaled it, then went straight to `sd update fabro-6a77 --status in_progress` (seq 115). It never ran the planner.md step-3-mandated `sd update <id> --description "<full corrected body>"` before claiming. The wrong path is now frozen in the closed seed row — the exact recurrence class the prompt's own fabro-05d0 motivation warns about, and the wrong-path hunt was most of this run's 143s planner burn.
**Change:** add a deterministic pre-claim check (extend `.fabro/scripts/dup-run-check.nu` or a sibling `claim-check.nu <id>`): resolve every repo path named in the seed body; on unresolvable paths, require the corrected body (or print it) before `sd update --status in_progress` is legal.
**New-seed justification:** no existing seed enforces the claim-time `--description` correction — fabro-19df only covers the safe round-trip mechanics for closure notes, fabro-d20f/fabro-c0ca shape briefs, fabro-3839 lints revisor intake; this run shows prompt-only mandates are skipped at `reasoning_effort=low`.
**Effect:** each stale-path seed gets corrected once in-tracker instead of being re-derived by every later reader/planner pass.

**3. Stop pouring the full `sd ready` firehose into the planner — 28KB, truncated.**
From run events seq 30: `sd ready --assignee fabro --limit 200` returned "200 ready issue(s)", 28,037 output bytes with `stdout_truncated: true` — the whole backlog enters the conversation (and the retained tail cuts into the list) just to learn that fabro-6a77 is top of the queue.
**Change:** `.fabro/workflows/develop/prompts/planner.md` sd table — top-N view first (`sd ready --first 10` / priority-filtered), full listing only when top candidates are unclaimable.
**Seed:** fabro-c3b4 (open; already absorbed fabro-9967). Tracker hygiene: fabro-66bc is a near-duplicate still open — close it against fabro-c3b4.
**Effect:** ~25KB less planner context per pass; the true head of the queue is never at truncation risk.

**4. Emit per-criterion check outputs in the verification-only evidence capture — the reviewer currently re-derives everything.**
From run events: the evidence capture (1.7KB, seq 132) shows `seed-work=0 files`, "(no seed-work files to diff)" — it contributed only the spec text. The reviewer then ran its own two rg batches (seqs 149–154) to verify all five brief criteria; its journal observation concedes the whole verification was self-performed ("confirmed via rg greps… no build or test rerun needed since greps matched the brief's exact assertions").
**Change:** `.fabro/workflows/develop/scripts/evidence.nu` — on verification-only claims, execute the brief's cheapest-first greps and inline their outputs as a per-criterion section of the capture.
**Seed:** fabro-f759 (open, verification-only variant; fabro-d89a is the implementer-run sibling).
**Effect:** the reviewer judges from the capture per its designed contract — shorter review passes and no dependence on the reviewer choosing to run tools.

**5. Expose `current_seed_id` in the `fabro_runs_list` projection — the in-flight guard ran on identical generic goals.**
From run events seq 39: the planner's in-flight check got 24 runs whose `goal` is the byte-identical generic string; it had to reason "only current run (my own) is running" and, for any mid-flight claim, would fall back to per-run journal greps (planner.md step 4's fallback dance).
**Change:** engine-side projection behind `fabro_runs_list` (as named by the seed) — add the claimed `current_seed_id` per run.
**Seed:** fabro-9372 (open).
**Effect:** the in-flight exclusion becomes one deterministic id comparison; removes the self-exclusion ambiguity and the journal-grep fallback entirely.

**6. Don't gate bookkeeping-only closure PRs — PR #187's whole diff is journal + one tracker row.**
From run events seq 179 (`run.completed` final_patch) and seq 178: the run's terminal diff is `.fabro/journal/<run>.jsonl` (+5 lines) plus the one-line fabro-6a77 status change in `.seeds/issues.jsonl` — zero source changes — yet PR #187 was created with auto-merge enabled and will ride the normal PR pipeline. Note fabro-9f97 (journal-only skip) explicitly does not cover this case because the tracker row is present.
**Change:** implement the gate skip for loop-asset-only run PRs (keep the PR — it delivers the tracker close), extending the loop-asset set to `.seeds/issues.jsonl` status-only rows.
**Seeds:** fabro-9495 (open) for the gate skip; fabro-b1d3 (open) to annotate the bookkeeping-only run in the Slack `run.completed` payload.
**Effect:** no project-gate/CI spend on no-op PRs, and terminal notifications tell the user this was a closure, not a change.

**7. Skip empty stage-journal records — 3 of 5 records this run are `{"data":{}}`.**
From checkpoint diffs (seqs 127/135/172): start, evidence, and closeout journal entries carry empty `data`, inflating every checkpoint diff and the evidence capture's loop-churn count (reported as `loop-churn=1 files +1/-1`).
**Change:** `.fabro/scripts/stage-journal.nu` — populate `data` (stage painpoints) or skip writing when empty.
**Seed:** fabro-850f (open).
**Effect:** less churn per checkpoint/PR diff; evidence churn counts stop counting noise.

**Not recommended (worked as designed, no change warranted):** the verification-only fast path (fabro-9d26 edge) skipped implementer+tester and still closed the seed with an approving first-pass review in 25s/$0.034 — the graph's cheapest correct cycle on record; the `2>/dev/null` probe at seq 43 and a suspected grep-dialect `\|` alternation in an rg pattern at seq 79 are both already covered by open fabro-b6f9 (probe discipline) and fold into recommendation 1's batching change rather than needing separate seeds.
