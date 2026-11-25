# ADR-0046: The Ask Fabro supervisor capability — one surface, two modes

## Status

Accepted (2026-11-18, grilled in session with the operator; Jev pre-screens
round 1 (surface model: second-persona-phased 1.0, overridden by the
operator for the widened-single-surface shape) and round 2 (mechanics:
operator-auth-per-session flip 1.0, soft-confirm merge 0.75,
invoked-only 1.0, per-run-widened anchor 0.98)). Amends ADR-0019's
documented read-only case: the run-local read-only chat posture becomes
the DEFAULT of a capability-stateful surface, not a permanent ceiling.

**Amended 2026-11-25 (work#1677, work#1678 — ADR-0046 phase 2, grilled
with the operator; Jev pre-screens: tools-only action path 1.00,
read-only-default 0.96 on the original framing, operator override to
full-access default for the runless case):** the "every NEW session
starts read-only" default is SUPERSEDED for the runless case. A runless
session — the global supervisor's conversation, created through
`POST /api/v1/sessions` — starts FULL ACCESS; the capability control
door stays, and run-scoped sessions keep the read-only default. The
supervisor keeps its OWN context: no continuation of run-scoped
sessions; session archaeology (search + compacted transcript load,
work#1679) replaces scope re-anchoring. Run content access rides
work#1680 (`fabro_run_files` + `fabro_run_diffs`). The supervisor's
action path is the audited tool registry over the minted
`agent:supervisor` token — no CLI shell; its pebble agent runs in the
deny-exec environment. The phase-1 mechanics below are unchanged.

## Context

Ask Fabro today is run-scoped and strictly read-only: granted exactly
`fabro_run_events`, `fabro_run_get`, `fabro_seeds_search`, a docs tool
and sandbox file reads (`ASK_FABRO_RUN_TOOL_NAMES`, sessions.rs); the
worker JWT rejects cross-run calls; `AskFabroToolPolicy` denies and
hides everything else ("Ask Fabro is read-only"). It executes as an
agentic pebble loop in the server process attached to the run's own
sandbox, one active turn per session, SecretRedactor on, context
compaction on, no cost caps. The CLI door is `fabro ask`.

The operator wants Ask Fabro to grow into the system's supervisor:
context over ALL runs, observations and work items; guidance during
builds in the UI; countermeasures when things go wrong; and a
CLI-reachable partner for the controlling session, which drives the
same fabro CLI today.

The capability tension is real and recorded (ADR-0019): agent surfaces
get least privilege; a supervisor holding `run_mark`, `run_merge` and
`work_close` can break the line. The operator's counter — accepted
into the record — is that the controlling session already holds exactly
that power, the line's one-executor principle is factually plural
(concurrent laps exist), and the deliberate quota posture is a 1M-token
window model with conversation reset as the lever, not a cost guard.

## Decision

ONE surface — Ask Fabro itself — with a capability control. Two modes:

- **Read-only (default).** Today's posture, unchanged: run-local read
  tools + sandbox file access. Every NEW session starts here.
- **Full access (operator flip).** The full run-tool registry:
  `run_mark`, `run_merge`, `run_create`, `work_*` (create/close/update/
  comments/next/radius/release), observation record, queue, search,
  wait, blob, interact — the same data sources the controlling session
  uses.

Mechanics:

1. **The flip is server-enforced** in the tool policy — a UI toggle
   alone would be decoration. Flipping requires the authenticated
   principal, persists per session (a new session starts read-only),
   and lands the auditable capability-change event in the SESSION LOG
   (a `run.session.capability_changed` kind through
   RunSessionEventStore — works for runless sessions), mirroring a
   platform record onto the run stream ONLY when the session is
   run-anchored.
   (Session events live in `run_session_events` and never enter run
   streams, and platform records are run-keyed — a global-page flip
   has no run to mirror onto; see the events strategy.)
2. **The CLI is a first-class door**: `fabro ask --capability full`
   performs the same server-enforced, audited flip. Anything the web
   UI can do, the fabro CLI can do.
3. **Merge carries a soft confirm**: full access includes `run_merge`,
   and the supervisor's system prompt MANDATES an explicit in-turn
   confirmation before calling it — the agent shows the PR, state and
   consequence and waits for the yes. The transcript is the audit.
   (run_merge is the one irreversible tool; a wrong park is
   reconciler-healable, a wrong merge is not.)
4. **No cost guard, deliberately.** The 1M-token window plus the
   conversation-reset door (a UI affordance starting a fresh session —
   the old one remains as the transcript) is the quota posture.
5. **Proactivity is invoked-only** in this phase: the supervisor acts
   within a turn — asked in the UI, or driven by the controlling
   session through `fabro ask`. An autonomous watcher is an explicit
   later decision, if ever, riding the watchdog gear's trigger rather
   than a new daemon.
6. **Session anchoring widens**: the per-run docked chat stays the
   entry point and gains the widened scope; a NEW global,
   viewport-filling chat page in the main menu (beside work and
   observations) hosts system-wide conversations where the conversation
   itself determines the context. Cross-surface continuity: the global
   page lists existing conversations (including per-run ones) and
   resumes them with full context — a session index read door replaces
   the per-run sessionStorage cache.

## Amendment note (ADR-0019)

Least privilege is preserved additively: nothing existing loses its
posture (the run-local chat defaults read-only; the in-sandbox stage
agents keep their full-registry grant behind the fs envelope). What
changes: the read-only case stops being a PERMANENT ceiling and becomes
the default state of a capability-stateful surface whose full state the
operator explicitly grants, per session, over authenticated doors,
with an audit event. Graded capability tiers, should fabro ever become
genuinely multi-user, ride the existing token scope machinery — no role
model is built now (today's production reality is a single operator).

## Consequences

- The supervisor under full access can mark runs, close work items and
  create runs — including while the line runs. Verdict collision with
  an in-flight adjudicator lap is an accepted residual (marks are
  reconciler-visible; work#1419's pin makes parks hold properly);
  `run_merge` is the one irreversible act and sits behind the soft
  confirm.
- Quota is spent deliberately unguarded; the reset door is the lever.
  Revisit the no-guard decision when the supervisor sees daily use.
- Migration footprint: capability-stateful tool policy (server),
  worker-token scope widening at full access, supervisor system prompt
  (merge soft-confirm mandate), the `run.session.capability_changed`
  event in the session log (RunSessionEventStore; a platform-record
  mirror on the run stream only when the session is run-anchored),
  global session index door + resume picker + reset door,
  global viewport chat route, `fabro ask --capability`. Phased as
  work items in project `fabro` (lane:develop).
- The vocabulary gains: **supervisor** (the full-access operating mode
  of Ask Fabro) and **capability control** (the per-session,
  server-enforced read-only/full switch) — recorded in GLOSSARY.md.
