# ADR-0047: The merger resolve push door (engine-mediated, run-branch-scoped)

## Status

Decided 2026-11-19 (operator approval in session; amends ADR-0019's
capability posture with a second instance of the decided merge-door
trust pattern).

## Context

The merger resolve pass (ADR-0039 stage E's conflict resolution)
cannot land its work: the merger sandbox holds no forge credential
(ADR-0019 posture — sandboxes get least privilege) and the server
exposes no run-branch push door (pushes happen solely server-side in
run_publish.rs). Every conflicted integration therefore parks
(work#1471's live specimen: run 01M7SQC6 / PR #607, resolved locally
but unpushable; the operator performed the keep-both resolution
manually three times the same day). The painpoint class floods the
triage — the append-only `.mulch/expertise/*.jsonl` logs conflict on
practically every integration where both a lap and a session appended.

## Decision

An engine-mediated push door: the server exposes a run-scoped endpoint
that pushes a named commit on the run's OWN branch
(`fabro/run/<id>`) to the forge — server-side credentials (the same
installation token run_publish uses), authorized to the merger
workflow exactly like the merge door (ADR-0039 stage E), the push
event recorded in the run stream as audit. The agent asks the door; it
never holds forge credentials.

Scoping fixed by this decision:

1. Workflow scope: the merger workflow only (the merge-door
   authorization shape).
2. Branch scope: the run's own branch — main, other branches, and
   force pushes are refused server-side.
3. Audit: a capability-class event lands in the run stream per push.
4. No raw git or forge token reaches the sandbox (the weaker token
   alternative is rejected).

## Alternatives rejected

- Contents-write token minted into the merger sandbox: weaker posture
  (direct forge power in the sandbox, no door to scope or audit).
- Operator confirm per push: maximal control, but reintroduces the
  operator into every conflict resolution — the workload this door
  exists to automate.
- No door: resolutions stay stranded, the resolve pass stays
  hand-gated, the painpoint class persists.

## Consequences

- The merger completes its ADR-0039 role for conflicts without
  operator intervention; the resolution content lands on the run
  branch where the normal adjudication/merge chain still judges it
  (a pushed resolution never bypasses review — it enables it).
- ADR-0019 gains its second instance of the decided pattern
  (server-side credentials, workflow-gated forge-write door, audited):
  the merge door was the first.
- The merger's resolve prompt (loop asset) must stop stating the false
  no-credential premise and teach the door instead — the correction
  rides the implementation.
- Abuse ceiling: a misbehaving merger can push arbitrary commits to
  its own run branch only; the trunk stays behind the adjudication/
  merge chain, and the audit event surfaces every push.
