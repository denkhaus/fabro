# Improve review — run 01M2QWF2RX4Q0BTB8AQWPH0K4M

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (12.6 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-17 14:49+0000 by revisor `fabro_ask`

---

All recommendations below are grounded in this run's actual behavior (run 01M2QWF2RX4Q0BTB8AQWPH0K4M, seed fabro-ca1f). Headline facts from run events: the claimed seed was **already green** — all 12 named tests passed on the untouched tree — yet the run spent 733 s / $0.264 discovering that. The implementer stage alone was 633 s wall (86% of run) and $0.132 (50% of cost), of which 465 s was two cold-compile reproduce probes (329 s + 136 s, events seq 89–97). The planner's entire pre-claim verification was an `rg -l` that test *names exist* (seq 50–53, 72 ms).

## Recommendations, by expected impact

**1. Dry-run the brief's cheapest repro probe before claiming — route Verification-only when it's green.**
Change: `.fabro/workflows/develop/prompts/planner.md` step 3/7 (and ultimately `.fabro/workflows/develop/scripts/planner-preflight.nu`): for bug seeds whose repro is a runnable test command, execute the cheapest named probe before `sd update ... in_progress`; a green result routes the existing "Verification-only" edge (planner→evidence→reviewer, already in the graph) instead of "Seed claimed".
What happened: the planner grepped that test names exist and claimed fabro-ca1f; the implementer then burned 465 s of cold compiles plus two full crate suites (956/956, 1536/1536) to learn the battery was already fixed upstream (implementer journal painpoint says exactly this: "~10 min of cold-build time here").
Expected effect: this run would have finished in ~3 min instead of 12; ~$0.10–0.13 saved per stale test-battery seed, and the implementer lap is reserved for real work.
Seed: **fabro-4be6** (dry-run every literal verification command before a brief ships) is the closest existing seed; its text only demands commands *run*, not that a green outcome change the route — so either fold the "green ⇒ Verification-only" routing rule into fabro-4be6 or file a one-line new seed, because no open seed covers outcome-based pre-claim routing (verified: tracker grep for repro/dry-run returns only 4be6/3805/4c81, all validity-scope).

**2. Top-N `sd ready` view instead of the 200-row firehose.**
Change: `.fabro/workflows/develop/prompts/planner.md` — `sd ready --first 10` (or `--priority high`), per the seed.
What happened: `sd ready --assignee fabro --limit 200` poured **28,822 bytes / 200 seed lines** into the planner conversation (seq 38–39) just to pick a top candidate that `output.preflight` had *already* delivered inline (5 candidates). The planner was the second-costliest stage: 56 s, $0.095, 38.6k input tokens.
Expected effect: ~29 KB less context per planning pass, fewer planner tokens/cost; composes with the preflight table the planner already trusts.
Seed: **fabro-c3b4** (open).

**3. Make the in-flight guard non-degraded: `current_seed_id` in the runs_list projection + `created_since` bound.**
Change: engine projection field per fabro-9372; add `created_since ≈ 48h` to the planner's step-4 call per fabro-6b58.
What happened (error handling): the in-flight check listed **41 runs** (~7 s tool time, seq 40–41), every goal the identical generic string with no seed id; the journal-grep fallback over the two failed runs' journals returned nothing, so the guard ran degraded — planner observation: "no in-flight exclusions applied". That is precisely the blind state that produced the fabro-22e4/fabro-6a5a double-picks cited in both seeds.
Expected effect: one projection field replaces the list+grep heuristic; the double-pick guard stops failing open exactly when a sibling run's PR sits unmerged.
Seeds: **fabro-9372** and **fabro-6b58** (both open).

**4. Sweep reviewer-journal "noted but not blocking" findings into seeds at closeout.**
Change: `.fabro/workflows/develop/scripts/closeout.nu` — extend the re-file mechanism to reviewer journal observations.
What happened: both the implementer and the reviewer flagged `server::tests::cancel_during_startup_persists_cancelled_reason` as timing-flaky (failed once under fail-fast, passed in the no-fail-fast re-run on an identical tree). fabro-ca1f then closed; the flake now lives only in `.fabro/journal/01M2QWF2RX4Q0BTB8AQWPH0K4M.jsonl` and mx-4b6685 — by the loop's own rule, a finding that lives only in a journal doesn't exist.
Expected effect: the flake becomes an open known-bug seed, so the next gate-red on it is matched deterministically by the gatebounce node instead of costing a fresh root-cause dig (cf. the 704 s / $0.38 re-derivation the gatebounce node was built to prevent).
Seed: **fabro-22fa** (open) — this run is fresh evidence for it.

**5. "Implemented (no changes)" edge from implementer straight to reviewer.**
Change: `.fabro/workflows/develop/workflow.fabro` — conditional edge skipping tester/evidence on a clean worktree.
What happened: the implementer shipped an empty diff (verification-only outcome by discovery), yet tester and evidence still ran. Cheap here (gate said "no crates touched", 4.9 s) only because nothing was touched — on any real tree the redundant gate pair costs minutes (the seed's own basis run measured this class).
Expected effect: removes one gate+capture pair per no-change pass; waste scales with gate weight, not with this run's luck.
Seed: **fabro-ff7a** (open).

**6. Long-stage heartbeat notification.**
Change: notifications block in the develop workflow settings — add a "stage exceeds N minutes" event alongside run.completed/run.failed.
What happened (UX): a 5.5-minute silent gap during the first cold compile (seq 89→90, 14:32:47→14:38:16) with zero external signal; the implementer held 86% of run wall. Anyone watching Slack or the UI saw nothing to act on.
Expected effect: mid-run visibility into single-stage cost concentration while intervention is still possible.
Seed: **fabro-b769** (open).

**7. Fix the latent type error in the preflight's mechanical close path.**
Change: `.fabro/workflows/develop/scripts/planner-preflight.nu:135` — initialize `mut closed = {seed: "", sha: ""}` (test non-empty at line 161) per the seed.
What happened (error handling): this run dodged the bug only because all five candidates were "clean", so the string-record assignment at line 152 never executed — I verified the `mut closed = {seed: null, sha: null}` declaration is still in the workspace file. The first genuinely already-landed candidate will hit `nu::shell::type_mismatch` inside the deterministic superseded-close, killing the node's one mechanical exit route exactly when fabro-a32f's saving (93.6 s / $0.143 per stale-tracker run) is supposed to materialize.
Expected effect: the "Already landed" auto-close works on first real use instead of dead-landing back to a planner lap.
Seed: **fabro-2ade** (open; this run is counter-evidence only for the "every run" claim, not the landed-path break).

**8. Per-stage cost/time table + painpoint digest in the PR body.**
Change: PR postlude composition — append the conclusion's stage table and journal painpoint digest to PR #217-class bodies.
What happened (UX): the fact that 86% of wall and half the cost sat in one stage — and *why* (a stale seed burning cold compiles) — is visible only in the run conclusion, not on PR #217 that the user merges.
Expected effect: at merge time the user sees build-dominated runs without opening the run dashboard.
Seed: **fabro-1409** (open).

Not inspected/limitations: I did not replay the full implementer transcript beyond event seq 100 (the remaining tool calls were the suite re-runs and `ml record`, summarized in the stage outcome); PR #217's merged state and Slack delivery were not verifiable from run events.
