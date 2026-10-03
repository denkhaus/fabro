# Improve review — run 01M2NP2W92HHF3QAH17F42GZ5K

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (30.7 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-16 18:56+0000 by revisor `fabro_ask`

---

All evidence gathered — stage timings/cost from the run conclusion, transcripts from run events, seed statuses from the workspace tracker (`.seeds/issues.jsonl`). Context: this run claimed **fabro-b959** (scheduler bug), all stages green first-pass, 30m40s wall, **$1.68 total** — of which the implementer was **$1.471 (87.6%) / 24 min (80%)** across 85 tool calls; reviewer approved in 23s; dup-run preflight did run (seq 124, ~16s into the stage — no issue there).

# Recommendations, ordered by expected impact

**1. Fix the evidence stage-renderer truncation — seed `fabro-meta-c9f2` (open, High).**
What happened: the evidence capture (13,497 B) reached the reviewer's preamble with **"(133 lines omitted)"** mid-diff (event seq 651/657); the reviewer self-healed by shelling out to `git diff 9e76d44..` against the claim base — its journal calls this out verbatim. Note the capture was *under* the 16 KB inline cap, so `fabro-cf3e` (raise `preamble_inline_max_kb`) would not have helped — the cut is the summary:high `tail_lines` cap, exactly `fabro-meta-c9f2`'s subject (its basis run saw 119 lines omitted; this run, 133).
Change: implement `fabro-meta-c9f2` in the engine's stage renderer (evidence delivery path).
Expected effect: the reviewer judges the complete capture every time — removes one fallback shell round-trip per review and the correctness risk of adjudicating a diff it hasn't fully seen.

**2. Hand planner-formed root-cause findings to the implementer — new seed.**
What happened: the planner's reasoning (events seq 85, 97) had the actual root cause at 18:02 — *"on revision change the cursor is rebuilt with next_due_at = next_occurrence(now)"* — then explicitly discarded it ("that's the implementer's job"). The implementer then re-derived precisely that mechanism inside the stage that consumed 80% of wall time and 87.6% of run cost (813s inference, 16 `read_file` + 58 shell calls).
Change: `.fabro/workflows/develop/prompts/planner.md` step 6 — when basis verification reveals a likely mechanism, add it to the brief as a bullet explicitly labeled *"unverified hypothesis (planner observation)"*, never as a requirement.
Expected effect: implementer exploration starts at the suspected mechanism on bug seeds — the single largest cost lever visible in this run.
New-seed justification: no existing seed covers planner→implementer hypothesis handoff; `fabro-1314` (open) governs brief mechanics in the *opposite* direction (outcomes not mechanisms), and a labeled-hypothesis bullet preserves that contract.

**3. Ship stale-mtime normalization for the implementer verify path — seed `fabro-56db` (open).**
What happened: lesson `mx-e7ae23`, recorded this run: after `edit_file` changes to `automation_scheduler.rs`, a `nextest` run reported a panic at a source line whose content no longer matched — cargo hadn't rebuilt ("0.5s Finished"), so **a correct fix read as failing for two consecutive test runs** inside the dominant stage.
Change: exactly what `fabro-56db` prescribes — `find lib -name '*.rs' ! -newermt 2000-01-01 -exec touch {} +` at qualitygate start **and mirrored in `implementer.md` step 4** (the seed names both sites; this run proves the class is live on the current toolchain image even though root-cause seed `fabro-22e4` is closed).
Expected effect: eliminates the stale-binary false-failure class from the most expensive stage at zero LLM cost.

**4. Planner firehose + reconnaissance batching — seeds `fabro-c3b4` (open) and `fabro-55a7` (open).**
What happened: the planner's first call (`sd ready --assignee fabro --limit 200`, event seq 30) poured **28,382 bytes / 200 seed rows** into context (stdout truncated) to learn which seed is top of the queue; the pass totalled 40,269 input tokens over 14 tool calls / 121.6s.
Change: `planner.md` step 1 per `fabro-c3b4` — top-N view (`sd ready --first 10` or priority filter), full listing only as fallback; batch the recon calls per `fabro-55a7`.
Expected effect: ~25 KB less planner context per pass, fewer round trips, shorter claim-to-dispatch window (which also narrows the duplicate-claim race the preflight guards).

**5. Match-count guard on pattern-wide transforms — new seed.**
What happened: implementer journal painpoint (verbatim): a broad `sed` matching `Some("default".to_string()),$` over `automation_scheduler.rs` **collaterally rewrote three unrelated test fixtures**, producing a confusing FK failure and a recovery round before it was noticed.
Change: `.fabro/workflows/develop/prompts/implementer.md` step 4 — extend the mechanical-transform rule with its inverse: count matches (`rg -c`) before any pattern-wide `sed`/`perl` transform; if the count exceeds the intended sites, use line-addressed sed or unique-context `edit_file`.
Expected effect: removes the collateral-rewrite → false-failure → diagnosis-detour chain observed here.
New-seed justification: no existing seed covers the over-broad direction of the transform rule (`fabro-1d94`/`fabro-4601` cover edit batching; `fabro-b6f9` covers probe hygiene).

**6. Store-layer environment validation on automation replace — new seed.**
What happened: implementer journal observation: `replace` targeting a nonexistent environment fails with a **bare SQLite FK error (code 787)** instead of a typed validation error — only the handler path validates (`resolve_automation_environment`), so direct store callers (tests, future code) get an opaque `Db` error; this cost a diagnosis detour during reproduction.
Change: `lib/components/fabro-automation/src/store.rs` `replace` path validates environment existence and returns a typed validation error mirroring the handler's.
Expected effect: actionable errors for every store caller; removes this reproduction detour class.
New-seed justification: no tracker seed covers FK-to-validation conversion for automation replace (`fabro-b959`, closed here, fixed the scheduler only; this was explicitly journaled as out of scope).

**7. Long-stage heartbeat — seed `fabro-b769` (open).**
What happened: the implementer ran 18:03:35→18:27:38 (24 of the run's ~31 minutes) with Slack notifications configured only for terminal events (run settings), so there was no external signal or intervention point during cost concentration.
Change: notifications block in `.fabro/workflows/develop/workflow.toml` per `fabro-b769` — add a "stage exceeds N minutes" heartbeat alongside terminal events.
Expected effect: mid-run visibility while cancelling is still possible.

**Not recommended for change** (checked, healthy this run): tester gate ran warm in 221s vs the 20m timeout; gate output (427 B) rendered fully in the reviewer preamble, so `fabro-750c` wasn't stressed; closeout (0.4s) and the one-seed-per-run exit worked exactly as designed.
