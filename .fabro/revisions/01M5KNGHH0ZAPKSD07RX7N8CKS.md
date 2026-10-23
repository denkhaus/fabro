# Revision — run 01M5KNGHH0ZAPKSD07RX7N8CKS

- status reviewed: succeeded (planner Blocked → deadlock exit; run diff journal-only, PR #487)
- review: .fabro/reviews/develop/01M5KNGHH0ZAPKSD07RX7N8CKS.md
- items filed: none — tracker binding outage still live (`no work project is bound to target repository denkhaus/fabro` re-verified at filing time); zero balance credit this pass
- balance: 0 non-exempt items filed / 0 — no credit this pass (no stale/superseded closes possible; tracker dead)
- basis: run 01M5KNGHH0ZAPKSD07RX7N8CKS, workflow version absent, commit 85334e301e033a72f44f79c7ecbb73d27e55a1c6
- revised_at_commit: 85334e301e033a72f44f79c7ecbb73d27e55a1c6 (ADR-0015: engine drift signal for later judgement)

## Findings

Closeout-race re-check ran degraded (tracker query failed): `fabro_work_list {"query": "01M5KNGHH0ZAPKSD07RX7N8CKS"}` failed with the binding outage; fallback `fabro_work_list` is unavailable for the same reason, and the analyst's degraded grep of `.seeds/issues.jsonl` + revision docs found 0 items citing the run. No filing race detected.

**1. Bind the work project's `targetRepo` to `denkhaus/fabro` (needs-user, server-side)** — overflow-dup: Bind the work project tracker to this repository (open in `.fabro/revisions/01M5K62554W7YCVZ7W4AT8YYWK.md`)

**2. Detect the dead tracker deterministically before the planner** — overflow
- overflow: Detect the dead tracker deterministically before the planner — add a work-tracker health probe to the shared guard script (`.fabro/workflows/develop/scripts/tracker-guard.nu`) and the `tracker_guard` node in `.fabro/workflows/loop/workflow.fabro`; on the unbound/outage error emit `preferred_next_label "Tracker outage"` with a `tracker_guard -> exit [x.kind="deadlock"]` edge, pinned by `.fabro/workflows/loop/scripts/graph-contract-smoke.nu`; effect: dead laps park in the ~4s deterministic prefix with zero planner tokens instead of 105.6s wall / $0.093 / 10 tool calls each (this run: tracker_guard answered "Tracker non-empty" from the legacy `.seeds` file while every `fabro_work_*` call failed). Cross-ref: open fabro-c03a is the unit that moves the deterministic scripts onto the same backend — the outage arm belongs in it; extension, not duplicate.

**3. Teach the planner that a config-outage tool error is terminal (prompt change)** — overflow
- overflow: Planner treats server-side config-outage tool errors as terminal — one paragraph in the tool reference of `.fabro/workflows/loop/prompts/planner.md`: a `fabro_work_*` error naming a server-side configuration condition is not retryable and not discoverable — journal the painpoint citing the revision doc, route `Blocked`, never guess project ids; effect: a future dead lap shrinks from ~105s / 10 calls to ~40s / 2 calls and stops credential/config guessing (this run: after the first error at 06:01:14 the planner ran 8 more turns / 9 more tool calls over ~75s rediscovering the documented outage). No existing item covers the planner's behavior on infrastructural tool errors (prompt contract, distinct from fabro-c03a's script work).

**4. Make a Blocked/deadlock exit visibly not-green** — overflow
- overflow: Carry the deadlock exit kind into the terminal run status — the deadlock exit kind travels into the terminal status payload and the `run.completed` event, rendered distinctly in the web header; effect: a lap that claimed, implemented, and closed nothing stops reading as a green `succeeded` completion in Slack `#dev-fabro` every 30 min (this run concluded `succeeded` after Blocked → deadlock, PR #487 diff is journal-only +5). Cross-ref: open fabro-0230 ("Surface root-cause error in terminal run status") is the same mechanism — extend it to deadlock exits; fabro-8cd3 (web rendering) and fabro-9e2c (terminal-event surfacing) are adjacent halves. Closed fabro-08b4 documented the class; the deadlock variant still lands green.

**5. Skip PR creation for a no-implementation pass** — overflow-dup: Skip or force-draft PR creation when the run diff is tracker-only (open in `.fabro/revisions/01M5KJ2NWJP1TM6JZZKQ2KHNX5.md`)

All five recommendations' full evidence lives in `.fabro/reviews/develop/01M5KNGHH0ZAPKSD07RX7N8CKS.md`; nothing filed this pass — the tracker binding outage (open overflow, finding 1) blocks every `fabro_work_*` write and is the only recommendation that restores the loop lane's purpose.
