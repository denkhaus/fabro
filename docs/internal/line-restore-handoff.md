# Handoff: restore the autonomous line on a loss-free foundation

Written 2026-10-20 by the integrate session (weblab). Audience: the next
host-side agent session. Chat language with the user: German. All times in
this document are UTC unless noted; user-facing times are Europe/Berlin.

> **CURRENT HANDOFF:** `docs/internal/work-tracker-handoff.md` (2026-10-20,
> later session). This document stays as the incident evidence; its plan is
> superseded.
>
> **SUPERSEDED IN PART (2026-10-20, later session).** The diagnosis below
> stands: landing is solved, assignment is broken, and the server is the
> assignment authority. The Phase-2 **store** form does not: the work-item
> store moves to one central private GitHub issue repository, the claim is
> a transactional server record projected as the `fabro:run:<id>` issue
> label, and the file tracker is frozen after migration. Read
> `docs/lab/adr/0032-central-work-item-tracker.md` and
> `docs/internal/central-work-tracker-migration.md` before executing any
> step below. The seeds-specific Phase-1 work (fabro-997d's preflight
> rewrite) is re-scoped there.

## Situation — why the line is OFF and must stay OFF

The user stopped both scheduled lanes (loop-fabro, revisor-fabro) at
~15:07 UTC 2026-10-20 (architect/conductor schedules were already off).
**The line stays off until the strategy below is implemented and the user
gives an explicit GO. Do not re-arm schedules.**

Reason (evidence, PRs #448-#458, one 40-minute window):
- 3 duplicate implementations / re-closures across loop and develop
  (#449 vs #448 on fabro-16a7; #455 vs #453 on fabro-7f58, identical
  file sets; #457 re-closed fabro-bdff 30 min after #453 closed it)
- 2 duplicate revisor passes (#454+#456 both revised run 01M5CR2B9;
  #456+#458 both revised 01M5CVG6XT9)
- 0 revisor-filed seeds (every finding stranded as "overflow" in
  revision markdown — no path into the tracker)
- one duplicate run reviewed as healthy (the revisor's own dup-run-check
  read its frozen tracker as OPEN)

Root cause, single: **every run claims from a frozen clone; nothing
server-side arbitrates assignment.** The ADR-0031 run-PR merger worked
flawlessly throughout (base-first, window-exact) — landing is solved;
assignment is the broken half.

Base state at handoff: 0 open run PRs, tracker 936 lines / 936 ids /
0 duplicates, base commit 6a6c1d30c6. All duplicate PRs were closed
unmerged with evidence comments; unique content was salvaged and landed.

## The strategy (user-decided direction, 2026-10-20)

> "wir brauchen dringend eine zentrale Instanz, die die Vergabe von Seeds
> an Runs steuert und pending pull requests im Auge hat. Einfach runs per
> scheduler zu starten funktioniert so nicht."

No new component: **the fabro server is the assignment authority.** It
already holds run status, the pull_request platform-record projection
(the merger reads it), and the planner's current_seed_id context. The
work is connecting them:

- **Phase 1 (re-arm precondition)**: fabro-997d (run_search PR link +
  planner-preflight arm-2 rewrite + open-PR claim exclusion), fabro-9372
  (current_seed_id in the run projection), fabro-4580 (backpressure: a
  lane fires only when its run PRs are drained).
- **Phase 2 (the central grant)**: fabro-b8b7 EPIC — transactional
  server-side seed assignment (claim record with run binding; tracker
  file becomes audit trail), PR watch over the claim+projection view,
  revisor pool = runs without markers INCLUDING markers in open PRs.
- **Phase 3 (optional, only when Phase 2 is stable)**: scheduler fires a
  queue driver instead of runs (pull-based lane start).

## Seed map (the strategy in tracker ids)

| id | owner | role |
|---|---|---|
| fabro-b8b7 | needs-user, P1 | EPIC central claim authority; forks A (record kind), B (grant API shape), C (requeue semantics) |
| fabro-4580 | @fabro, P2 | backpressure fire guard; fork: cap 0 (recommended) vs N |
| fabro-997d | @fabro, P2 | server-state in-flight exclusion; carries the full 2026-10-20 race evidence |
| fabro-9372 | open, P2 | current_seed_id in run projection (pre-existing engine seed) |
| fabro-6cbf | needs-user, P2 | revisor overflow stranding; forks A-D (backlog lane / auto-file / priority-file / human) |
| fabro-065a | @fabro, P2 | verify.nu exit contract + test-names blank-line drop (stranded finding, filed) |
| fabro-bf3c | @loop, P2 | seed-spec-paths into the implementer's cheap lane (stranded finding, filed) |
| fabro-a827, fabro-17fb, fabro-801e | @loop, P3 | gate lint diff-scoping, reviewer read dedup, date-pin sweep (stranded findings, filed) |

The five "stranded finding" seeds are normal queue work, NOT re-arm
blockers. The blockers are 997d + 9372 + 4580 + the decisions below.

## Steps for the next agent, in order

1. **Orient.** `git pull --ff-only`; verify the line is off
   (`FABRO_SERVER=https://mirtuell.net fabro automations list` — every
   schedule AUS); `gh pr list --state open` = 0 run PRs; tracker
   duplicate count 0. Read this file and the seed map above.
2. **Decision round with the user** (needs-user seeds; decisions go INTO
   the seed bodies via `seeds update` BEFORE any implementation, then
   drop the needs-user label): b8b7 forks A/B/C; 4580 cap (0 vs N);
   6cbf path A-D.
3. **Implement Phase 1** while the line stays off. Two safe modes:
   - host-side for small, resource-light changes (see host facts below),
   - or MANUAL single fires: one seed per manual run (the api trigger
     stays enabled; `fabro automations fire` or `fabro run <workflow>`),
     pipeline drained before and after, landing via the merger window or
     `gh pr merge --squash --delete-branch` after the dogfood-gate is
     green. Never two runs in flight; never re-arm schedules for this.
   - JSONL discipline on every merge (count duplicate ids; closed-wins
     union). Duplicate containment: same-files twin of a landed PR ->
     close unmerged citing it (ADR-0031(5) class). Branch repair:
     detach on the run branch, rebase onto origin/denkhaus, resolve
     tracker-only conflicts closed-wins, keep base versions of
     duplicate review/revision files, `push --force-with-lease`, wait
     for the dogfood-gate, squash-merge.
4. **Re-arm checklist** (all boxes, then ask the user for GO):
   - [ ] 997d, 9372, 4580 landed and green at HEAD
   - [ ] b8b7 forks decided; Phase 2 broken into implementable seed units
         (or landed if cheap)
   - [ ] 6cbf path decided and, if the chosen path requires it, the
         revisor pool marker semantics fixed
   - [ ] backpressure active with the decided cap
   - [ ] user GO
   Then re-arm revisor-fabro and loop-fabro only (develop fires through
   the revisor cascade; architect/conductor stay off unless the user
   says otherwise). Acceptance bar for the first 24h: zero uncontained
   duplicates (dup-run-check verdicts and PR closes tell you).

## Operational facts

- Production server: `FABRO_SERVER=https://mirtuell.net` (OAuth context
  lives in ~/.fabro/auth.json). The CLI default target localhost:32276
  is dead on the workstations.
- CLI skew: the weblab CLI (0.362.0-fork.1) cannot parse newer server
  responses (unknown field `model_providers`) — `fabro automations
  list/pause` fail with "Invalid Response Payload". Fix: `fabro upgrade`
  on the workstation (pending on weblab as of this handoff). Diagnose
  skew first with `fabro version` (client vs server).
- Host landscape: mirtuell = permanently on, production + line host,
  sufficient resources (line seeds need NO resource scoping). weblab and
  v1000 = the user's two workstations, used alternately; weblab has
  FEWER resources than v1000 — no comprehensive local builds on weblab
  (targeted crate tests locally, heavy verification on mirtuell). Check
  `hostname` before heavy local work.
- The merger (ADR-0031) is server-internal, NOT schedule-gated: it keeps
  landing open run PRs in minutes :10-:25 and :40-:55. That is desired —
  leave it alone.
- Merge windows :10-:25/:40-:55 vs claim windows :00-:04 (ticks :00/:30
  when armed). Never push to base while run PRs are open; a drained
  window is the safe push slot.
- Left-running agent sessions with line-watch heartbeats can re-fire a
  parked line through the ALWAYS-ENABLED api trigger (trigger_id
  "manual"). Check `agent_observe.list_agents()` for running siblings
  and their heartbeats before assuming the line is quiet, and after any
  unattributed fire.
- Tracker: `.seeds/issues.jsonl`, CLI `seeds` (create/update/close/show/
  search — search is AND-strict, one keyword per query).
- All user-facing times: Europe/Berlin.
