# ADR-0030: Agent-refined run titles

Date: 2026-10-18
Status: accepted (user decision, grill-with-docs session 46)

## Context

Run titles are per-type boilerplate (the workflow goal), so board tiles,
runs list, run detail and Slack terminal notifications repeat one string
per lane and runs are indistinguishable at a glance (fabro-9711, user
demand). The engine already LLM-generates a contextual title at run
creation (fire-and-forget, `run_title_generation.rs`) and natively
generates PR titles/descriptions; a manual rename API exists with a
"manually set stays" guard. Seed-specific knowledge (which seed a run
works on) appears only when the planner claims — after creation.

## Decision

Run titles stay LLM-generated, with TWO generators:

1. The existing creation-time engine generation is unchanged (runs that
   never claim still get a contextual title; fallback before any
   generation is the workflow's generic goal title).
2. A new run tool `fabro_run_set_title` lets the agent that defines the
   work — the planner that claims the seed (develop, loop, revisor) —
   refine the run title specifically for the claimed seed. The tool is
   the refinement seam; the agent composes the title (few sentences,
   seed-specific, not generic).

- Default chain: workflow-generic title -> engine LLM title at creation
  -> agent refinement at claim. Nothing after an agent write overrides
  it automatically; manual renames keep the existing guard semantics
  (last write wins, no provenance machinery — Q11).
- Cap stays `MAX_RUN_TITLE_CHARS = 100`; tiles are small. If more space
  is ever needed, change the constant once (user decision 2026-10-18).
- The tool accepts an OPTIONAL run selector beyond the pinned prompts'
  own-run calls (no argument): a surface without a session run (a future
  conductor titling a child) needs it; the stage-session dispatcher
  injects the session's current run when the call carries none, so the
  planner contract stays selector-free.
- Initial tool policies: develop planner, loop planner, revisor
  (conductor is a later nice-to-have pin). Registered like the other
  run tools (node `x.fabro_tools` pins + server-side policy allowlists;
  ADR-0019 satisfied by this recorded user decision).
- Landed as ONE unit with fabro-3a5e (Kanban tile identifiers: claimed
  seed id, run id, start date) — same context source, same API surface.

## Rejected alternatives

- Server-side watcher on the projection reacting to the claim context
  (rejected 2026-10-18: the agent knows the work best; a watcher
  composes from a template instead of understanding).
- Engine-side envelope-merge write in Petri (rejected: ADR-0027 freeze
  cost for a write the agent can make itself).
- Prompt-only instruction to call the rename API (rejected:
  deterministic-first; a tool with a typed contract is the stable seam).
- Client-side composition of claimed-seed context per surface
  (rejected: Slack text is server-side — two composition points,
  drift).
- Raising the title cap to 200 (rejected: tiles are small; revisit by
  changing one constant).
