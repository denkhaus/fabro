# Improve review — run 01M5KQ7FB5CZ9JHATY12SC7ASS

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: blocked (planner parked at human interview 06:31:34Z after ~49s / $0.059, revisor pass — reason and cost in run detail)
- generated: 2026-10-23 09:24+0200 by revisor `fabro_ask`

---

## What actually happened in this run (evidence base)

- `env_guard` → `tracker_guard` → `preflight` all green in ~4s total (42ms / 303ms / 3.6s, $0 — from run events). `tracker_guard` returned **"Tracker non-empty"** by reading the **legacy `seeds` CLI** (`tracker-guard.nu:198-199`).
- `planner@1` then called `fabro_work_next {"lane":"loop"}` → error: **"no work project is bound to target repository denkhaus/fabro"** (event seq 92-93). It then **guessed** `project: "denkhaus/work"` → "Invalid Request: conversion to `ListWorkItemsProjectId` failed" (seq 101-102), grepped `.fabro/config*`, read `.fabro/project.toml` (which has no project binding and stale `fabro-sh/fabro` daytona env), grepped develop scripts — ~49s, 8 tool calls (3 errors), 19.3k input + 112k cache-read tokens, **$0.059** — then recalled prior run **#487 blocked identically** and opened a human interview at 06:31:34. The run is **still blocked** there.
- 4× `unsupported_control` warnings: the zai protocol **drops the tool-result error flag**, so the model received the error text unflagged (from run events).
- Docs: `internal/work-tracker-handoff.md` — the ADR-0032 migration is mid-flight, the line was to stay **OFF until Phase 6 + user GO**, and the work project (`fabro` → `denkhaus/work`) is configured **only on the local server**, not on production `mirtuell.net` where this schedule fired.

Dedup searches run (all open-status, `fabro_seeds_search`): `project`, `guard tracker`, `targetRepo` (0 hits), `bound`, `needs-user`, `error flag`, `interview`, `automation schedule re-arm` (0 hits), `preflight legend`, `notification blocked` (0 hits).

## Recommendations, by expected impact

**1. Stop the guaranteed-dead scheduled fires: bind the project on production or pause the schedule.** — Tracker item: **fabro-a33e** (EPIC: central work-item tracker; its Phase 2 user action + Phase 6 re-arm gate cover exactly this).
Every `loop-fabro` fire on mirtuell.net is structurally unable to claim (no bound work project), so each one burns ~$0.06 and parks on a human question — this run and #487 prove the pattern. Concrete change: on mirtuell.net, either `PUT /api/v1/projects/...` with `targetRepo denkhaus/fabro`, or disable the `loop-fabro` automation until the epic's Phase 6 GO. Effect: zero further dead fires.

**2. Graph design: make `tracker_guard` fail-closed on the unbound-project state, like `env_guard` does for placement.** — Covered by **fabro-a33e** (Phase 3 explicitly lists rewriting `tracker-guard`/`planner-preflight` onto the project API).
Evidence: the guard's "Tracker non-empty" green light came from the frozen legacy file while the claim path needs the central tracker — so the guard waved the run through into a 49s/$0.059 LLM dead end, and `preflight` wasted another 3.6s producing 5 "clean" candidates (fabro-9337…) that could never be granted. Concrete change: add an unbound-binding arm to `.fabro/workflows/develop/scripts/tracker-guard.nu` (shared by loop via `--assignee loop`) emitting a new routing label wired in `.fabro/workflows/loop/workflow.fabro` to the existing deadlock exit, mirroring the `env_guard`/fabro-3ab2 fail-closed teaching-error pattern. Effect: dead fires park in <1s at $0 with an error naming the fix — no LLM, no interview.

**3. Prompting: add a no-guessing branch for the binding error in the planner's tool table.** — Covered by **fabro-a33e** (Phase 3 prompt rewrite; loop planner is in its call-site inventory).
Evidence: `.fabro/workflows/loop/prompts/planner.md:15` says "Omit `project` — it resolves to this run's project" with **no failure branch** — the agent improvised a fabricated project id and a 40s repo hunt. Concrete change: one line in the tool reference: "on the 'no work project is bound' error, never guess a project id and never search the repo — route `Blocked` with failure_reason naming the server-side fix." This is also the stopgap the prompt's own standing policy prefers to avoid ("mechanically-checkable invariant → deterministic check, never prose" — rec 2 is the real fix). Effect: worst case collapses from ~50s improvised hunt + HITL to one failed call + deadlock exit.

**4. Error handling: stop dropping the tool-result error flag on the zai profile.** — Tracker item: **fabro-c98c** (open bug: "zai profile still drops the tool-result error flag — unsupported_control WARN on production loop runs"); **fabro-e486** covers the WARN spam.
Evidence: this run had 3 `is_error: true` tool completions and 4 `unsupported_control` warnings; the model got the binding error as unflagged text and treated it as something to work around. Concrete change (engine-side, product lane — not loop-editable): in the openai-compat adapter, when the protocol lacks the error flag, render `is_error` into the result content (e.g. `ERROR: <text>`). Effect: models reliably recognize tool failures; shorter, saner recovery loops.

**5. UX: make a blocked scheduled run actually visible.** — Adjacent item: **fabro-07cf** (open: per-scope interview question channel). New-item justification for the notification half: no open seed covers notifying on blocked/HITL state (`notification blocked` → 0 hits); engine lifecycle Slack events are only `run.started/completed/failed` (`lib/apps/fabro-server/src/server.rs:582`, `.fabro/workflows/loop/workflow.toml:102`), and this run has `interviews.provider = null`, so the pending question sits only in the web interview dock (per the Interviews doc) — nobody was pinged.
Concrete change: set the loop automation's interview Slack channel (`#dev-fabro`) — the fabro-07cf per-scope work — and/or add to `planner.md`: "unattended schedule fire + no interview channel → route `Blocked`, don't `request_user_input`." Effect: a silently parked run becomes a Slack-visible one.

**6. Minor efficiency: trim the preflight payload inlined into the planner.** — New-item justification: **fabro-ead4** (which *added* the verdict legend) is closed; nothing open covers slimming the inlined output (`preflight legend` → only that closed seed).
Evidence: preflight stdout was 3,590 bytes, roughly half of which is the repeated advisory `legend` plus ~20 null fields per candidate — all inlined verbatim into the planner prompt (visible in the planner UserInput event). Concrete change: in `.fabro/workflows/develop/scripts/planner-preflight.nu`, emit null-suppressed candidates and move `legend` out of `context_updates` (keep it in `--help`). Effect: ~1.5KB/≈400 tokens saved per planner pass, less misread risk, no behavior change.

**Working well — don't touch:** the `env_guard` fail-closed pattern (42ms, zero-cost), the fast empty-tracker exit design, and the stage journal (clean one-record-per-stage, from `.fabro/journal/01M5KQ7FB5CZ9JHATY12SC7ASS.jsonl`). The blocker is configuration and transition sequencing (recs 1-2), not the gate battery itself.
