# Revision — run 01M5KNGHH0ZAPKSD07RX7N8CKS

- status reviewed: succeeded (deadlock-routed Blocked; journal-only diff, PR #487 merged)
- review: .fabro/reviews/develop/01M5KNGHH0ZAPKSD07RX7N8CKS.md
- items filed: none — tracker binding outage persists (every `fabro_work_*` call fails "no work project is bound to target repository denkhaus/fabro"); zero balance credit and no filing path, findings journaled as overflow for the next pass
- balance: 0 / 0 — no credit this pass
- basis: run 01M5KNGHH0ZAPKSD07RX7N8CKS, workflow version absent (not journaled; select named loop baseline 36a97a61…), commit 01fc7f59204f58946e39545449a5421f9cbc6a8c
- revised_at_commit: 01fc7f59204f58946e39545449a5421f9cbc6a8c (ADR-0015: engine drift signal for later judgement)

## Findings

Withheld as duplicate (analyst step, recorded here for the trail): the deadlock-exit journal-only PR suppression recommendation — duplicate_of open fabro-5a3f (accept-vs-suppress fork governs journal-only PRs in every lane; this run is loop-lane regression evidence vs closed fabro-9f97, whose journal-only skip did not cover the deadlock path).

Filing attempt: `fabro_work_list {"limit": 5}` re-check at filing time failed with the same binding error ("no work project is bound to target repository denkhaus/fabro"); degraded closeout-race grep of `.seeds/issues.jsonl` for the run id found 0 matches. Dedupe for the findings below therefore ran against the analyst's tracker searches (review lines 23, 28, 38) plus the open overflow ledger — no open or closed item names any of them.

**1. Add a central-tracker health probe + "Tracker unreachable" exit arm to the deterministic prefix** — overflow

- overflow: Add a central-tracker health probe + "Tracker unreachable" exit arm — in `.fabro/workflows/develop/scripts/tracker-guard.nu` (shared by the loop's `tracker_guard` node), add an arm probing the central tracker binding (the surface `fabro_work_next` needs); on the binding error emit `preferred_next_label "Tracker unreachable"` and wire `tracker_guard -> exit [x.kind="soft"]` in `.fabro/workflows/loop/workflow.fabro` (and develop's graph); effect: the scheduled loop stops spending a full planner pass ($0.093 + ~2 min per fire, recurring every automation tick while the outage lasts) to rediscover a mechanically detectable outage — blocked passes become a ~4s zero-token exit. Dedupe: analyst searches `targetRepo`/`outage`/`bind`/`unbound`/`work project`/`guard` found nothing (fabro-a33e is migration build-out, fabro-e1ee is preflight-failure soft-exit); not present in the open overflow ledger.

**2. Teach the planner prompt to fail fast on the known binding error** — overflow

- overflow: Teach the planner prompt to fail fast on the known binding error — in `.fabro/workflows/loop/prompts/planner.md` GRANT step, add a rule: if `fabro_work_next` returns the "no work project is bound…" error, do not retry or search for the project id — route Blocked immediately citing the revisions doc (this run: after identifying the outage, the planner still ran a second `fabro_work_next` plus 6 shell calls for ~66s; rule must match on error text, which is stable, because the provider dropped the tool-error flag); effect: a blocked planner pass collapses from ~105s/$0.093 to roughly the 29s initial-reasoning minimum. Dedupe: analyst searches `outage`/`blocked`/`degraded` found nothing (fabro-2afd closed/absent preflight, fabro-5b0a degraded-check surfacing, fabro-d62c run-level tools); not in the open overflow ledger.

**3. Make deadlock-exited runs read as blocked, not "succeeded"** — overflow

- overflow: Make deadlock-exited runs read as blocked, not "succeeded" — classify deadlock (`x.kind="deadlock"`) exits as a distinct terminal status at the engine's conclusion fold and render it distinctly in the web header; effect: the needs-human signal the graph comment promises actually reaches the human — this run routed Blocked on the deadlock edge yet concluded `succeeded/completed` and Slack `#dev-fabro` got a green `run.completed`, so outage-class deadlocks hide inside green notifications. Cross-ref: open fabro-8cd3 renders soft_stop/deadlock distinctly (inverse case — deadlock rendered as succeeded; thematic overlap, different mechanism, close nothing); engine classification fabro-08b (closed) evidently didn't cover this edge kind. Not in the open overflow ledger.
