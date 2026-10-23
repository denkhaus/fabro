# Improve review — run 01M5KNGHH0ZAPKSD07RX7N8CKS

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (planner 105.6s of a ≈3m11s run, revisor pass — reason and cost in run detail)
- generated: 2026-10-23 09:38+0200 by revisor `fabro_ask`

---

## What actually happened in this run (evidence base)

From run events and the run summary: this was the **first loop run on the Phase‑3 graph** (base `59c56331a` = PR #484, "Rewrite loop workflow prompts onto fabro_work_* tools"). The deterministic prefix worked (`env_guard` 42 ms → `tracker_guard` 383 ms "Tracker non-empty" → `preflight` 3.5 s, 5 "clean" candidates: fabro‑9337/3ec9/9dd6/d4b0/c03a). Then **every `fabro_work_*` call failed** with "no work project is bound to target repository denkhaus/fabro" (planner tool calls at 06:01:14 and 06:01:57). The planner spent the run's entire LLM budget — **105.6 s wall / 104.5 s inference / $0.093, 29 s to first token, 9 LLM turns, 10 tool calls** — rediscovering an outage documented before the run in workspace file `.fabro/revisions/01M5K62554W7YCVZ7W4AT8YYWK.md` (outage spans revisor runs `01M5K208R20CB3007RH84H39NM` and `01M5K625555R46VT94D1XGXENC`), then routed `Blocked` → deadlock exit. The run still concluded **"succeeded"** and opened **PR #487** whose whole diff is the journal file (1 file, +5/−0, from the run's diff summary). The `loop@fabro` schedule fires every 30 min (three loop runs visible 05:00/05:30/06:00 in the planner's own `fabro_run_search`), so every lap repeats this until fixed.

Tracker searches I ran to dedupe: `outage`, `targetRepo`, `work project bound`, `fabro-c03a`, `fabro-9337`, `notification` (open), `blocked PR`, `pull request` (open), `empty diff`, `run status` (open).

## Recommendations, by expected impact

**1. Bind the work project's `targetRepo` to `denkhaus/fabro` (or name the project id in the run context) — needs-user, server-side.**
Evidence: the failure text itself names the fix; the planner proved the id is undiscoverable from inside the run (it grepped `.fabro`, `tracker-guard.nu`, prior journals, `git log`, and resorted to "try a few plausible ids" — which ADR‑0032 forbids as a tracker surface). PR #484 shipped prompts onto tools with no binding, and the revisor can't file this item because its filing surface is the same dead tracker (its 06:00 run filed 0 items). Effect: unblocks the entire loop lane — the queue demonstrably has live candidates.
→ **New-item justification:** searches `targetRepo` and `work project bound` returned 0 seeds — no tracker item anywhere covers the server-side project↔repo binding. (I could not inspect the server config from this workspace.)

**2. Detect the dead tracker deterministically *before* the planner.**
Evidence: `tracker_guard` answered "Tracker non-empty" from the **legacy `.seeds` file** (`seeds list --assignee loop`, fail-open to non-empty — from workspace file `.fabro/workflows/develop/scripts/tracker-guard.nu`), so the guard green-lit a planner whose claim surface was dead. Change: in the shared guard script + the `tracker_guard` node in `.fabro/workflows/loop/workflow.fabro`, add a work-tracker health probe; on the unbound/outage error emit `preferred_next_label "Tracker outage"` with a new `tracker_guard -> exit [x.kind="deadlock"]` edge, pinned by `.fabro/workflows/loop/scripts/graph-contract-smoke.nu`. Effect: dead laps park in the ~4 s deterministic prefix with zero planner tokens — exactly the economics the guard already delivers for "Tracker empty" — instead of 105 s/$0.09 each.
→ **Item: fabro-c03a** (open, assignee loop, "Phase 3: shared scripts onto server API") — the unit that moves the deterministic scripts onto the same backend as the planner; the outage arm belongs in it.

**3. Teach the planner that a config-outage tool error is terminal (prompt change).**
Evidence: after the first error (06:01:14) the planner ran **8 more turns / 9 more tool calls over ~75 s** (rg over `.fabro`, reading the guard script, `fabro_run_search`, two prior-run journals, `git log`, a second `fabro_work_next`, a `fabro_work_list`) to conclude what the error string plus the outage note it found 11 s in already said. Change: one paragraph in the tool reference of `.fabro/workflows/loop/prompts/planner.md`: a `fabro_work_*` error naming a server-side configuration condition is not retryable and not discoverable — journal the painpoint citing the revision doc, route `Blocked`, never guess project ids. Effect: any future dead lap shrinks from ~105 s/10 calls to ~40 s/2 calls, and stops credential/config guessing.
→ **New-item justification:** no open item covers the planner's behavior on infrastructural tool errors (this is prompt contract, distinct from fabro-c03a's script work).

**4. Make a Blocked/deadlock exit visibly not-green.**
Evidence: this run is `status: succeeded` after `Blocked` → deadlock exit, and the notification config sends `run.completed` to Slack `#dev-fabro` — a lap that claimed, implemented, and closed nothing reads as a green completion, recurring every 30 min. (Closed fabro‑08b4 documented this class; the deadlock variant still lands green.) Change: carry the deadlock exit kind into the terminal status payload and the `run.completed` event; render distinctly in the web header. Effect: the outage is visible as blocked/red on the very first dead lap instead of never.
→ **Item: fabro-0230** (open, "Surface root-cause error in terminal run status") — extend it to deadlock exits; **fabro-8cd3** (web rendering) and **fabro-9e2c** (terminal-event surfacing) are the adjacent halves.

**5. Don't open a PR for a no-implementation pass.**
Evidence: PR #487's entire diff is the run journal (1 file, +5/−0), and PR creation spent a glm‑4.7 pass (06:02:35–06:03:49) writing it up — pure review noise, one per dead lap. Change: in the loop `workflow.toml` `pull_request` handling / engine PR creation, skip (or auto-close as "blocked, no changes") when the run's diff is journal-only or the exit is a deadlock. Effect: dead laps stop generating human-visible PR cleanup work.
→ **New-item justification:** `pull request` (open) and `empty diff` searches surface only projection/backfill items (fabro-4bb7, fabro-dc81) and the inverse problem (fabro-1437: green cycles publishing *nothing*) — nothing covers gating PR creation on an implementation diff.

One caution on ordering: items 2–5 are mitigations; until #1 lands, the loop lane produces zero work-item progress per lap regardless — #1 is the only recommendation that restores the workflow's purpose.
