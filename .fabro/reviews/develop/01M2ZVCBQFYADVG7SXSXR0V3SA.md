# Improve review — run 01M2ZVCBQFYADVG7SXSXR0V3SA

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: failed (2.2 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-20 17:17+0000 by revisor `fabro_ask`

---

## What actually happened in this run (evidence base)

From run events and the final conclusion: `tracker_guard` (241 ms) and `preflight` (1.57 s, verdict `clean` for `seeds-e218`) succeeded; `planner@1` claimed `seeds-e218` and shipped a valid brief (42.7 s, $0.105). Then `claim_check` failed deterministically 3× with `stdin value is not a seed id: seeds-e218` (87/81/405 ms), bouncing the planner twice more (`planner@2` $0.035/31.9 s, `planner@3` $0.108/42.4 s) until the engine's deterministic-signature breaker (limit 3) killed the run at 16:48:39. Run totals: 133.4 s wall, $0.249, 117k input tokens — **100% of LLM spend was the planner, and visits 2+3 ($0.143, 74.3 s) were pure waste**: the planner correctly diagnosed the bug at 16:47:29 (planner@2, `sed -n 1,60p` of the script) and journaled the exact fix, then had no route but to re-claim and re-ship the same brief.

Root cause, from workspace file `.fabro/workflows/develop/scripts/claim-check.nu` line 21: `if not ($seed_id | str starts-with "fabro-")` — the script hardcodes the origin repo's `fabro-` prefix; this repo's ids are `seeds-` (PROJECT_FACTS even uses `seeds-e218` as its example). The workflow port adapted the prompts but not this script.

Tracker check (from `.seeds/issues.jsonl`): exactly three seeds exist — `seeds-e218` (in_progress), `seeds-facc` (open, blockedBy e218), `seeds-3791` (open, blockedby facc). All three are product seeds; none targets loop assets. (`seeds-facc`'s "tracker scripts still work when pointed at the new binary" clause is about the sd→seeds binary cutover, not this bug.) **So every recommendation below carries a new-seed justification.**

## Recommendations, by expected impact

**1. Fix the prefix hardcode in `claim-check.nu` — the run-killer.**
- Change: `.fabro/workflows/develop/scripts/claim-check.nu` line 21 (and the header comment line 10, which also says "fabro- seed id prefix"): replace `str starts-with "fabro-"` with a generic seed-id pattern, e.g. `$seed_id =~ '^[a-z][a-z0-9]*-[0-9a-f]{4}$'` (matching the `<project>-<hex4>` format PROJECT_FACTS already documents).
- Expected effect: the exact failure class that killed this run disappears; identical runs route planner → implementer instead of dying at 133 s/$0.25 with zero seed work done.
- New-seed justification: the tracker's only seeds (seeds-e218/facc/3791) are product work; none covers loop-asset fixes under `.fabro/`.

**2. Stop routing deterministic `claim_check` failures back to the LLM planner.**
- What happened: the retry edge `claim_check -> planner ["Claim contract failed, re-plan"]` in `.fabro/workflows/develop/workflow.fabro` exists for *intermittent model-side* context_updates drops (~50%, per the fabro-c42f comment). Here it re-entered an LLM twice on a *deterministic script* failure no planner can fix — $0.143 (57.6% of run cost), 74.3 s, and two no-op `sd update` re-claims (updatedAt churn 16:47:03 → 16:47:39 → 16:48:21 visible in checkpoint diffs).
- Change: in `workflow.fabro`, add a soft exit from `claim_check` for `failure_class="deterministic"` (mirroring the existing `evidence -> exit [kind=soft, label="Capture failed (next run re-enters)"]`), keeping the planner-retry edge only for non-deterministic failures — or at minimum bound the retry to one lap.
- Expected effect: any future deterministic contract failure parks in <1 s/$0 instead of ~75 s/$0.14; the engine's 3× breaker becomes a backstop, not the primary cost control.
- New-seed justification: no existing seed covers graph/edge changes; all three tracker seeds are product seeds.

**3. Make failed runs stop blocking the only ready seed for 60 minutes.**
- What happened: the run died at 16:48:39, leaving `seeds-e218` in_progress with a pushed run branch. Per `.fabro/workflows/develop/scripts/planner-preflight.nu` lines 103–114, a failed branch tip counts as terminal only after `TERMINAL_GRACE_MIN_DEFAULT = 60` minutes — so any develop run launched before ~17:48 sees `seeds-e218` as `in_flight` and skips it, and the other two seeds are blockedBy it: the whole line parks. The tracker-guard fallback is worse: its terminality test requires the journal's last record to be `closeout` (tracker-guard.nu line 172), which a failed run never has, so requeue waits the full 6 h `FABRO_GUARD_STALE_HOURS` (6.0 visible in this run's tracker_guard output).
- Change: in `planner-preflight.nu` `terminal-tip?`, treat a tip whose failure was signature-latched by the engine breaker (the run conclusion's `deterministic failure cycle` message) as terminal immediately, bypassing the 60-min grace; optionally have the new soft exit from rec 2 roll back the claim (`sd update <id> --status open`).
- Expected effect: after a failed run, the next run re-claims and proceeds immediately instead of parking up to 1 h (or 6 h if no run is launched).
- New-seed justification: preflight/tracker-guard semantics are loop assets; no product seed covers them.

**4. Bound the planner's `fabro_runs_list` call with `created_since` — a recurring ~15 s tax per planner visit.**
- What happened: planner@1's `fabro_runs_list` call ran 16:46:36.294 → 16:46:51.347 (15.05 s, run events seq 48–49) returning **121 runs** unbounded — mostly hours-old terminal runs from both repos; planner@3 paid it again. ~30 s ≈ 23% of this run's wall time for a check that only needs *non-terminal* runs, and whose branch-scan half is already done deterministically by preflight (which answered `in_flight: false` in 1.57 s).
- Change: `.fabro/workflows/develop/prompts/planner.md`, step 4 (IN-FLIGHT EXCLUSION): add one clause — pass `created_since` bounded to the stale threshold (e.g. last 6 h) when calling `fabro_runs_list`; the tool already supports it (its own description says so).
- Expected effect: ~15 s and a much smaller context payload saved on every planner visit, in every run.
- New-seed justification: prompt change to a loop asset; no existing seed covers it.

**5. Move the seed-id format invariant into the planner output schema (prevention at output time).**
- What happened: the malformed-prefix mismatch survived schema validation and a green preflight, and only exploded *after* the tracker claim had landed. The planner's own standing policy (fabro-9ec3, quoted in planner.md) says mechanically-checkable invariants belong in `planner-preflight.nu` or the planner output schema — this one is in neither; it's in a post-hoc node with a wrong constant.
- Change: `.fabro/workflows/develop/schemas/planner-output.schema.json`: add `"current_seed_id": { "type": "string", "pattern": "^[a-z][a-z0-9]*-[0-9a-f]{4}$" }` under `context_updates.properties` — the same teeth pattern the schema already uses to ban gate commands in `current_seed_brief`.
- Expected effect: a malformed id burns one `output_retries` round at the planner (seconds, *before* the sd claim) instead of a post-claim graph cycle; complements rec 1, which still guards the stdin plumbing.
- New-seed justification: schema change to a loop asset; no existing seed covers it.

**6. Harvest journal painpoints from failed runs — the fix was already written down 70 s after the first failure.**
- What happened: planner@2 diagnosed the exact bug and journaled it twice (16:47:50 and 16:48:36) with file, line, evidence run id, and fix idea — the ideal painpoint record. The run then failed; whether any improve/revisor pass ever reads a *failed* run's journal is not visible in this run's events.
- Change: whatever workflow scans `.fabro/journal/` for painpoints (the graph comment says "the improve workflow scans run branches for journals") must include terminal *failed* runs' journals — arguably first, since that's where loop-breaking bugs surface.
- Expected effect: the one-line fix in rec 1 would have landed next cycle even without anyone re-reading this run manually.
- New-seed justification: improve-loop harvesting behavior; the three product seeds don't cover it.

Could not inspect: whether a subsequent develop/improve run has already consumed this journal (no events after 17:12:30 are visible in this run's log), and the revisor workflow's actual scan filter.
