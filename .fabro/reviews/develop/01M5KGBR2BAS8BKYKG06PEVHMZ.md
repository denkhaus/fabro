# Improve review — run 01M5KGBR2BAS8BKYKG06PEVHMZ

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (96s, revisor pass — reason and cost in run detail)
- generated: 2026-10-23 07:04+0200 by revisor `fabro_ask`

---

## What this run actually was

A 96-second, $0.084 "verification-only" lap on seed **fabro-30a5** — a seed whose Action line is literally *"none"*. The planner proved its sole criterion (dependency fabro-4653 still open) via one recorded check, the reviewer approved with zero tool calls, and then **closeout parked the seed anyway** — yet the run exited green through the "Seed closed" edge (from run events: closeout `exit_status 0` → edge 25 → exit). Cost split: planner $0.0597/44.4s (71% of spend, 20.7k input + 94.3k cache-read tokens), reviewer $0.0245/17.3s, all five command stages combined ~5.1s. Tracker searches run for dedup: `closeout verification-only`, `closeout park`, `no-op seed`, `top-N ready`, `title`, `closeout`, `deferred follow-up`, `verification-only reviewer`, `park seed closed green`, `placeholder interpolation`, `none residual filing`.

## Recommendations, by expected impact

**1. Closeout must close reviewer-approved verification-only seeds instead of parking them.**
Evidence: closeout output "PARK — … shows no implementing change for the seed's demand … Seed left OPEN" (run events, closeout stage) — the diff-anchored park heuristic is designed for implement laps and can't see recorded checks as proof. The reviewer explicitly said "the deterministic Closeout can close fabro-30a5." Worse, tracker_guard *requeued a stale fabro-30a5 claim* at run start (stale >6h) — this seed is already on a repeat lap, so the loop will burn ~$0.08 + 96s + a no-op PR (#480, diff = tracker row + journal only) every cycle until a human intervenes.
Change: in `.fabro/workflows/develop/scripts/closeout.nu`, accept `review_verdict=approved` + no implementer visit as closable when the evidence capture's "recorded checks" section proves the criteria; park only when neither diff nor recorded checks do.
Effect: fabro-30a5 closes on the next lap; the recurring no-op lap disappears.
*Justification (new item): searches `closeout verification-only`, `closeout park`, `verification-only` found no open item — fabro-9e2c covers only surfacing the park state, not the park-vs-close decision.*

**2. Stop filing "none"-action residual seeds at closeout.**
Evidence: the planner's own journal this run — "fabro-30a5 is a zero-action residual … consider not filing 'none' follow-up seeds at all"; the tracker already holds another one (fabro-7ece, "Deferred follow-up from fabro-a8d5: none"). Each such seed forces a full LLM lap like this one.
Change: in `closeout.nu`'s residual sweep, when the disclosed follow-up action is literally "none", append the note to the origin seed instead of filing a new seed.
Effect: eliminates this entire class of verification-only laps at the source.
*Justification (new item): searches `none residual filing`, `deferred follow-up` surface only the pattern-instance seeds, no process item.*

**3. A parked closeout must not exit green through the "Seed closed" edge.**
Evidence: closeout printed PARK, exited 0, and the unconditional `closeout -> exit [label="Seed closed"]` edge fired; the run concluded `succeeded(completed)` with the seed open, and #dev-fabro got a run.completed notification for it (from run events, edge 25 → exit visit).
Change: implement **fabro-9e2c** (open) the graph-side way — `closeout.nu` emits `preferred_label="Seed parked"`, and `.fabro/workflows/loop/workflow.fabro` (plus develop's copy) gets a distinct soft edge `closeout -> exit [x.kind="soft", label="Seed parked"]`, mirroring the existing planner `outcome=failed` soft-edge pattern already in the graph.
Effect: parked laps read resumable/soft, count toward the breaker, and stop masquerading as completions in the terminal event and notifications.

**4. Fix the closeout park message: unsubstituted `($seed_id)` and the line printed twice.**
Evidence: stdout shows `closeout: PARK — ($seed_id) NOT closed …` verbatim, duplicated, in both the stream events and `step.finished` (run events seq 260/261/263).
Change: in `closeout.nu`, interpolate the actual seed id (stdin `current_seed_id` is already the node's input) and emit the line once.
Effect: the terminal event, Slack notification, and journal name the parked seed instead of a literal placeholder, and don't repeat themselves — humans can act without opening the run.
*Justification (new item): search `closeout` (13 items) and `placeholder interpolation` (0) — nothing covers the broken interpolation or duplicate print.*

**5. Give the planner a top-N ready view instead of the 65-issue firehose.**
Evidence: the planner's first shell call returned all 65 ready issues, 9.4KB retained in context (run events, tool call `seeds ready --assignee loop --limit 200`), yet the decision used only the top candidate ("The preflight's top candidate is fabro-30a5"). The planner stage is 71% of run cost and 46% of wall time.
Change: implement **fabro-c3b4** (open, loop-assigned) — top-N sorted view in `.fabro/workflows/develop/scripts/planner-preflight.nu`/planner prompt; note fabro-66bc and fabro-9967 (both closed) didn't stick because the prompt still mandates `--limit 200`.
Effect: materially smaller planner conversation (the largest single cost pool), same claim quality when the preflight ordering is trusted.

**6. Make the run-title budget stick in the planner prompt.**
Evidence: first `fabro_run_set_title` call rejected — "run title must be at most 100 characters" — then a second LLM turn and call succeeded ~7s later (run events, planner tool stats: 2 calls, 1 error; the prompt's ADR-0030 block already warns about the cap but the model still overshot).
Change: extend **fabro-b529** (open — the 100-char cap is too tight) to the loop lane, or until the cap moves, add a hard "≤90 chars, front-load the demand" instruction with an example in `.fabro/workflows/loop/prompts/planner.md`.
Effect: one tool call and one less model turn per run (~7s and a wasted request on every titled pass).

One thing I could not inspect: the closeout/evidence script sources themselves (`closeout.nu` lives under `.fabro/` in the run worktree, not this analyst workspace), so the proposed insertion points are named from the graph source and stage transcripts, not read from the files.
