# Improve review — run 01M2N4XEB1XQW4466CXB57YJ4K

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (6.8 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-16 13:13+0000 by revisor `fabro_ask`

---

All evidence gathered — run events/checkpoints, stage transcripts, journals, worker log, and the seed tracker (`.seeds/issues.jsonl`). Basis: run 01M2N4XEB1XQW4466CXB57YJ4K implemented seed **fabro-9d26** (verification-only fast path) in 6m18s wall / $0.388 total: planner 100s/$0.104 (27%), implementer 225s/$0.239 (62%), tester 3.7s, reviewer 34s/$0.044. Recommendations ordered by expected impact:

**1. Make the evidence capture survive the stage renderer — seed fabro-meta-c9f2 (open, P1).**
What happened: the reviewer's journal painpoint (from run events, reviewer@1) reports the evidence capture arrived as "(60 lines omitted)" at the head of the tester/evidence block, "hiding most of the workflow.fabro and planner.md diff hunks; I had to re-verify via shell grep/sed." The capture is the review input; the renderer cut exactly its critical-first sections. This is the 4th recorded occurrence on that seed. Change: rearchitect `.fabro/workflows/develop/scripts/evidence.nu` per the seed's fix plan (diff tail-budgeted to the `summary:high` tail_lines window, or engine-side full-output rendering for the reviewer node). Expected effect: reviewer approves from visible evidence with 0 shell re-verification calls and the verification-blocked/recycle risk class disappears.

**2. Stop tasking the implementer with re-proving settled engine facts — seed fabro-fb19 (open).**
What happened: the brief bullet "verify no unconditional planner edge can shadow it" sent the implementer reading `lib/components/fabro-workflow/src/graph/routing.rs`, its tests, and `context.rs` (per its PASS report) to re-verify what is a standing engine guarantee — inside the stage that consumed 62% of run cost ($0.239) with only 2.0s of tool time. Change: `.fabro/workflows/develop/prompts/planner.md` brief-shaping rule per fabro-fb19: inline the 2–3 load-bearing engine facts (conditions outrank unconditional edges; allow-keys is a whitelist) into PROJECT_FACTS and have briefs cite them as settled instead of re-tasking verification. Expected effect: removes the source-reading detour from the costliest stage — tens of seconds / ~$0.05–0.10 per loop-asset seed.

**3. Add the available-binaries line to PROJECT_FACTS — seed fabro-0586 (open).**
What happened: the brief's verification tier named `fabro validate` ("per CLI docs"); the implementer found "no prebuilt target/debug/fabro binary and no graphviz dot in the sandbox" (its journal observation) and had to invent python3 DOT-structural checks — pure fallback reasoning in the 62%-cost stage. Change: one line in `.fabro/workflows/develop/prompts/project-facts.md` listing toolchain binaries (no `fabro` CLI, no `dot`; `python3`, `rg`, `sd`, `just`, `nu` present). Expected effect: planner stops prescribing unavailable verification commands; implementer skips the discovery/invention turns.

**4. Trim planner reconnaissance — seeds fabro-66bc + fabro-6b58 (both open; batching via fabro-55a7).**
What happened: planner burned 96s inference / $0.104 (27% of run) on a fully-resolved seed. Its first call `sd ready --assignee fabro --limit 200` poured 200 seeds / 27,944 bytes into context (event seq 30, stdout truncated) and it picked the first line; the unbounded `fabro_runs_list` returned 21 runs of full JSON and the planner manually deduced "the only non-terminal run is this run itself" (seq 33–36). Change: planner.md command table → top-N priority-sorted `sd ready` (fabro-66bc); step 4 → `created_since ≈ 48h` + explicit self-exclusion line (fabro-6b58); batch the probes into one shell call (fabro-55a7). Expected effect: ~25KB less planner context per run, cheaper turns, narrower claim-to-dispatch race window.

**5. Re-verify the pipeline-progress header fix — seed fabro-9e8b (open).**
What happened: this run re-confirms the counter-evidence — implementer@1's prompt read "Pipeline progress: 0 of 7 stages completed" with planner already done, and reviewer@1 read "2 of 7" with 4 non-meta stages completed (from the stage.prompt events). Change: verification seed against the engine progress projection in `lib/` named by the seed. Expected effect: honest mid-run numbers in every stage prompt (and squash trailer) instead of numbers only correct at termination.

**6. Prune the now-dead verification-only section in the implementer prompt — NEW SEED.**
Justification: no existing seed covers it — the condition was created by this run's own merge (fabro-9d26's fast path), and fabro-578a targets brief command scope, not dead prompt text. What happened: implementer's journal observation — "`implementer.md` lines 133–135 ('## Verification-only briefs') are now dead text: verification-only claims never reach the implementer on the new fabro-9d26 route." Change: prune/repurpose that section in `.fabro/workflows/develop/prompts/implementer.md` to a one-line pointer (verification-only routes planner→evidence→reviewer). Expected effect: smaller implementer prompt and no stale instructions that would mislead if the routing ever regresses.

**7. Carry tool errors through the provider protocol — seed fabro-b09c (open).**
What happened: the worker log shows 8 `unsupported_control` warnings during implementer ("this provider protocol does not support the tool result error flag") around its 1 errored shell call of 21 — errors reach the model as sniffed stdout, inviting blind re-runs. Change: provider-protocol error flag per the seed. Expected effect: the agent sees failed tool calls as first-class errors; fewer wasted retry turns in the costliest stage.

**8. Make PR-body generation non-strict — seed fabro-41b1 (open); plus fabro-a701 for log noise.**
What happened: worker log 13:08:03 — "PR content structured generation failed; retrying once without strict JSON output … the model did not return a JSON document" (recovered; PR #181 exists). Same log shows 6 ERROR lines for the absent optional `.codex/instructions.md` (fabro-a701, info-not-error). Change: non-strict PR postlude per fabro-41b1; downgrade the memory-file miss per fabro-a701. Expected effect: no retry latency/risk on the publish path; clean warn/error views for real diagnostics.

Not inspected: PR #181's final merge state (projection showed `state: null` at snapshot time) and the full 11.8KB evidence blob (assessed via its byte count, the reviewer's truncated view, and the reviewer's journal).
