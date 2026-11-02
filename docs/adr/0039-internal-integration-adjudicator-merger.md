# ADR-0039: Internal integration — the adjudicator–merger architecture

## Status

Accepted (2026-11-01). Supersedes the filing arm of ADR-0038 stage 4
(the run-end observation filing hook strips before the next deploy
window). Extends ADR-0038's observation records into the orchestration
substrate. Subsumes the dogfood-gate retirement epic
(denkhaus/work#1081) as its premise. Epic carrier:
denkhaus/work#1107.

## Context

Three incident classes proved that a run cannot manage its own
aftermath:

- A `deferred-action: none` journal observation became a junk tracker
  item (fabro-30a5): prose markers parsed by string guards.
- A marker vocabulary mismatch (`deferred-action:` vs `exemption:`)
  silently dropped a filing; the TypeScript client drifted out of sync
  with nothing remembering the arm (run 01M673H9C4).
- The run-end filing hook's LLM distillation drops observations
  intermittently (a known ~40 percent flake).

Two structural faults sit under them:

- **Conflict of interest.** The run that produced the work also closes
  the work item it worked on. Every discipline gate (the
  closed-with-no-patch gate fabro-02c4 and its siblings) is a patch
  over this one structure.
- **The closure radius is a fiction inside a run.** The radius spans
  observations from ALL runs of an item (`runs_for_item`); a single
  run cannot oversee it. The closeout even files follow-ups unlinked
  so the radius gate in `work close` does not refuse.

Deploy windows cost hours end to end; the line deploys nothing it
will not use.

## Decision

1. **The develop run is the unit of work.** Its end product is a pull
   request carrying the performed work. It never closes its item,
   never files follow-ups, never judges itself.
2. **GitHub automerge and GitHub CI gates retire completely.**
   Verification and integration run inside fabro. The pull request
   stays an artefact (visibility, review surface), never a gate.
3. **The adjudicator** (a scheduler-driven serial workflow, HITL)
   reviews the observations of all runs one after another; reconciles
   painpoints, markers, and notes; files follow-up work items; watches
   the closure radius; asks the human operator for policy decisions;
   and runs in TWO passes: pass one marks `merge: enabled` on the run
   record; pass two closes the item once the merger marked
   `closable: enabled`.
4. **The merger** integrates every merge-enabled run of a
   repo/project, resolves merge conflicts, and marks `closable`. It
   owns technical integration, never the content verdict.
5. **Work-item administration** guarantees single assignment and lock
   discipline. **The lock is derived** — an item is locked while it
   has an active claim whose run is not yet merged (claim lifecycle
   `awaiting_merge`/`landed` already carries the halves). The picker
   refuses locked items; channel authorization refuses foreign
   closes. The lock span is claim to merge-complete.
6. **Orchestration fields** (glossary): two enum fields on the run
   record, one writer each — `merge: pending|parked|enabled|refused`
   (the adjudicator writes) and
   `closable: pending|parked|enabled|refused` (the merger writes).
   `parked` in both fields is an operator escalation (adjudication
   policy fork; unresolvable merge conflict). Dataset fields are the
   ONLY coupling between workflows — no messaging conventions.
7. **Mechanical fast path, agent where judgment is needed**: the
   trivial case (no markers, no painpoints, empty radius) closes
   mechanically without an LLM call; the agent pass runs only where
   judgment exists.
8. **HITL shape: corridor plus park.** The pass runs autonomously for
   clear cases. A true policy fork triggers an operator ask; no answer
   within the pass parks the run and pings Slack; the next pass
   resumes. The serial tick never blocks on a human.
9. **Three serial scheduler gears** (item-driven develop, run-driven
   adjudicator, merge-marker-driven merger), each with its own tick
   and `on_overlap: skip`, coupled only through orchestration fields.
   A dead-man watch alarms when a gear makes no progress. The
   conductor's serialize-at-run-complete fault (#1080) dissolves into
   this model.
10. **The closeout stage becomes the reporter**: a facts-only step
    reporting patch visibility and Dockerfile touches as observation
    records. The run reports facts; it never judges. The name
    `closeout` retires with its close authority.
11. **Markers are data fields on the observation record**: field
    `followup`, values `unfinished | exempt | minor` (the prose
    journal marker sweeps retire with the closeout's authority).

## Consequences

- The fork.30 window carries no server payload (all server-side
  commits since fork.29 are hook substance) and slips until the
  adjudicator chain is ready; the stage-4 live proof and the
  `filing_enabled` settings checklist are cancelled. Stage C later
  added payload again (engine quartet bindings, the observation CLI,
  reporter facts); the user decision of the sprint-73 grill
  (2026-11-01) extends the slip: NOTHING deploys to mirtuell while the
  chain is in flight — the local fabro carries the current build and
  local testing covers it; the window runs once the work is complete.
- The transition is ONE cutover: adjudicator live, reporter renamed,
  closeout authority gone, hook stripped — lanes are empty, the
  adjudication queue starts empty.
- Stage 5 (radius view, denkhaus/work#1082) and #1098 shift behind
  the adjudicator.
- The revisor stays retired (ADR-0038 stage 4); its outbox/reconciler
  discipline returns as the adjudicator's fast-path reliability
  pattern.
- Upstream posture unchanged (ADR-0024): this is fork-private
  orchestration architecture.
