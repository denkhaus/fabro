# Improve review — run 01M5K4BB81H36V7B4QHF9NTDS2

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: failed (~5.5 min, revisor pass — reason and cost in run detail)
- generated: 2026-10-23 03:18+0200 by revisor `fabro_ask`

---

All evidence is in. The decisive facts (from run events, stage transcripts, and the run journal):

- **The whole run was eaten by one bounce loop.** planner→claim_check cycled 3× (planner@1 $0.173/166.9s, @2 $0.088/70.9s, @3 $0.068/58.1s; total $0.329 / 292s inference of a 330s run). The seed body shown by `seeds show` inside claim-check was **byte-identical in all three failures** — the blocking path `scripts/closeout-race.nu` sits inside the body's own correction parenthetical ("(Correction 2026-10-23: scripts/closeout-race.nu no longer exists…)"), which claim-check's rooted-path extractor (`ROOTED_PREFIXES` includes `scripts`) treats as an existence citation because there's a creation-intent window but no *retirement*-intent window (from `.fabro/workflows/develop/scripts/claim-check.nu`, sourced window logic in `anchor_check.nu`).
- **The planners "fixed" the wrong thing twice** — both later visits verified "all ten cited develop files resolve" (the Files brace-list) while the gate kept extracting the parenthetical mention; claim-check's bounce message lists only the path, never the body line it came from (stage outputs claim_check@1–@3).
- **Terminal state:** the circuit-breaker middleware blocked routing at event seq 577 — "deterministic failure cycle detected … repeated 3 times (limit 3)" — and its signature string is a mid-token slice ("acker-guard,closeout,evidence,gate-bounce}.nu…") that became the run's `failure_reason`, i.e. what Slack #dev-fabro and the run page display.
- **Journal painpoints** (`.fabro/journal/01M5K4BB81H36V7B4QHF9NTDS2.jsonl`): preflight marked fabro-d48f `in_flight_run=01M5K208QW…` although that run was terminal (succeeded 00:49, 11 min earlier).
- **Tool friction:** `fabro_run_search(status=[queued,…])` → "unknown run status `queued`" (planner@1: 4 calls, 2 errors); `seeds ready` returned a 9.7KB / 67-seed firehose; `fabro_run_set_title` ran 4 calls with 3 errors in each re-claim visit.

Tracker searches run: `d48f`, `claim-check`, `in-flight`, `run_search`, `claim`, `deadlock`, `stale path citation`, `set_title`.

---

## Recommendations, by expected impact

**1. claim-check: add a retirement-intent window and quote the offending body line.**
What happened: 3 identical bounces, 2 wasted planner sessions ($0.156, ~130s), zero forward progress — because a path the body explicitly says "no longer exists" is treated as an existence citation, and the bounce payload doesn't show *which* mention produced the path.
Change: in `.fabro/workflows/develop/scripts/claim-check.nu` + the shared window logic in `anchor_check.nu` — (a) mirror `creation-named?` with a retirement window (rooted path immediately followed by "no longer exists"/"never existed"/"retired" ⇒ non-citation); (b) in the blocking list, print the quoted body snippet each path was extracted from.
Expected effect: this bounce class disappears outright; genuine stale citations get fixed in one planner pass instead of three.
Seed: **new-seed justification** — searches `claim-check`, `stale path citation`, `claim` found no open seed; fabro-4c81 (closed) built the gate, none covers its retirement-mention false-block or payload line-quoting.

**2. planner-preflight: in-flight predicate must filter to non-terminal run statuses.**
What happened: preflight flagged the top-priority seed `in_flight_run=01M5K208QW…` for a run that had succeeded 11 minutes earlier (journal visit-1 painpoint); the planner burned `fabro_run_search` calls re-adjudicating a non-issue.
Change: in `.fabro/workflows/develop/scripts/planner-preflight.nu`, the in-flight arm counts only non-terminal run statuses.
Expected effect: removes a false positive that has now cost adjudication effort in two consecutive runs and risks wrongly skipping the highest-priority seed.
Seed: **fabro-997d** (open — "Deterministic in-flight exclusion from server state: run_search PR link + preflight arm 2 rewrite"); adjacent planner-side bound already tracked as **fabro-6b58**.

**3. Engine: circuit-breaker failure signatures are byte-sliced mid-token.**
What happened: the run's terminal `failure_reason` — propagated to the conclusion, the run page, and the Slack `run.failed` notification — begins "…status 1\nacker-guard,closeout,evidence,gate-bounce}.nu…" because the breaker's deterministic signature (event seq 577) truncates the message inside a brace-expansion token.
Change: engine-side, wherever the breaker builds its `node|class|message` signature: cut at a line/token boundary and lead with the node label (e.g. "claim_check: stale seed body — scripts/closeout-race.nu"), not a mid-body slice.
Expected effect: humans triaging #dev-fabro see "claim-check stale-body bounce ×3" instead of word salad; this failure surface is engine/product code, so route to @fabro per the lane's surface router.
Seed: **new-seed justification** — no existing seed (searches `deadlock`, `stale path citation`, `set_title` found none); engine surface uncovered by the loop-lane backlog.

**4. Graph: give claim_check the generation-capped deadlock exit that tester/reviewer already have.**
What happened: the graph caps `nodes.tester.generation >= 3` and `nodes.reviewer.generation >= 3` with `x.kind="deadlock"` exits, but claim_check's only failure edge bounces back to planner (max_visits=20); the engine breaker ended the cycle only after a *third* full planner session, as an undifferentiated `failed(workflow_error)` instead of the designed "seed stays open for a human" deadlock exit.
Change: in `.fabro/workflows/loop/workflow.fabro` (and develop's graph): `claim_check -> exit [x.kind="deadlock", label="Claim-contract deadlock (needs a human)", condition="nodes.claim_check.generation >= 2"]`.
Expected effect: the toxic loop stops one planner visit earlier (~$0.07 / ~65s per occurrence) and terminates as a labeled, human-handoff deadlock rather than an engine error.
Seed: **new-seed justification** — exit kinds came from closed fabro-288d; no open seed adds a claim_check generation cap (search `deadlock`).

**5. Planner prompt: verify what the gate sees, not what you meant — re-grep the body after `--set-description`.**
What happened: planner@2 and @3 each claimed "body corrected, all cited files resolve" and routed Seed claimed again; neither re-checked the stored description for rooted path tokens inside parentheticals (their `--set-description` re-emissions changed nothing the gate reads — bodies identical across all three `seeds show` outputs). Related friction: each re-claim visit re-attempted `fabro_run_set_title` 4× with 3 errors against the "one title write per run" rule.
Change: in `.fabro/workflows/loop/prompts/planner.md` step 3: after any `seeds update --set-description`, run `seeds show <id> --format json` and grep the description for every rooted token (`scripts/…`, `.fabro/…`, `lib/…`) before routing Seed claimed — retirement mentions get deleted, never annotated; and skip the title call on re-claim visits (already set).
Expected effect: bounce corrections land in one pass even before rec 1 ships; journal painpoint "apply body corrections in the same pass" becomes a checked step instead of a reminder. (The lane's standing policy puts mechanically-checkable invariants in scripts — that's rec 1; this is the behavioral complement.)
Seed: **new-seed justification** — probe-discipline seeds fabro-b6f9/fabro-8d4c cover greps/sed, none covers the post-correction verify loop; `set_title` search found no seed.

**6. fabro_run_search: make the first in-flight query valid — prompt names the statuses, tool error names the enum.**
What happened: planner@1's in-flight exclusion call used `status=["queued",…]` and errored "unknown run status `queued`" (2 of 4 `fabro_run_search` calls failed), costing retry turns at reasoning_effort=low.
Change: engine side is already tracked — **fabro-1cde** (open: "fabro_run_search: unknown-status error must name the valid RunStatusKind values"); complement it in `.fabro/workflows/loop/prompts/planner.md` step 5 / PROJECT_FACTS by listing the valid non-terminal status values verbatim.
Expected effect: the in-flight guard fires correctly on the first call each run instead of after an error-and-retry cycle.

**7. Cut the claim-phase firehose the planner pays for on every visit.**
What happened: `seeds ready --assignee loop --limit 200` returned 67 seeds / ~9.7KB into a `reasoning_effort=low` session; planner@1 alone ran 23 tool calls / 166.9s / $0.173 — the most expensive single stage of the run — much of it ordering a queue whose top candidate was already known from the preflight table.
Change: already filed — **fabro-c3b4** (open, @loop: "Use a top-N sd ready view in the planner instead of the full firehose") plus **fabro-a08a** (open, @loop: visit-1 fresh-cycle line to skip dead context reads); land them in `.fabro/workflows/loop/prompts/planner.md`.
Expected effect: shorter, cheaper planner visits (this run's claim phase was ~85% of total cost) and less context pollution for the adjudication steps that matter.

---

**What already worked well (keep as-is):** the zero-token guard chain (env_guard 50ms, tracker_guard 370ms, preflight 3.6s, all deterministic), the circuit breaker as the last-resort backstop, and claim-check's fail-open posture — the failure was concentrated entirely in the planner↔claim_check loop, which recs 1, 4 and 5 dismantle from three sides (gate semantics, graph cap, prompt verification).

**Not inspectable from here:** the live description body of fabro-d48f (the tracker search returns rows without bodies), so "body unchanged across all three bounces" rests on the three `seeds show` captures inside the claim_check stage outputs rather than a direct tracker read.
