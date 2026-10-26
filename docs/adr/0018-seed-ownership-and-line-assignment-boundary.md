# ADR-0018: Seed ownership and the autonomous line's assignment boundary

- Status: Accepted
- Date: 2026-09-07
- Deciders: user + agent (fabro session, grill)
- Related: fabro-5e09, PR #38, ADR-0015 (serialized line), ADR-0012 (dogfooding)

## Context

The develop workflow's planner consumed the full `sd ready` firehose: every
open, unblocked seed was a candidate. On 2026-09-07 an unattended scheduled
fire created a parallel conductor pass (no `on_overlap` set), and grill-session
seeds needed a mutual-block sentinel hack (fabro-919b/fabro-80cc) to stay out
of the planner's reach. The user demanded a deterministic restriction of the
line to seeds explicitly meant for it — and challenged the first design for
letting the line curate its own backlog ("woher weiß fabro, welche seeds für
ihn bestimmt sind?").

## Decision

1. **Assignee is the ownership switch.** The develop planner's ONLY candidate
   source is `sd ready --assignee fabro --limit 200`. The CLI pre-filters at
   the data level: the planner model never sees unassigned or user-owned
   seeds, so no prompt can override what it cannot see. A seed id named in a
   run goal is honored only when its assignee is `fabro` — naming is a
   request, not an override of the ownership decision.
2. **Nobody inside the autonomous line owns work.** The revisor files seeds
   (unassigned) from its reviews; it never assigns — reviewing is not owning.
   The line never self-assigns, including for urgent fixes.
3. **Execution ownership is decided jointly by user + agent.** The recurring
   forum is the cycle report (and grill sessions): new seeds are listed with
   a recommendation; an `@fabro` assignment made by the agent is a proposal
   the user can veto anytime by reassigning.
4. **Opt-in, fail-closed.** Nothing belongs to the line until `@fabro` is
   explicitly set. An empty filtered view is a legitimate park ("tracker
   empty"), never a reason to fall back to unassigned seeds or invent work.
5. **Emergency channel stays outside the picker.** When the line or engine is
   down, the agent repairs directly with the user's knowledge (standing
   directive 2026-09-07); the documenting seed is filed retroactively. There
   is no self-service emergency class for the revisor or the line.

## Consequences

- The ownership boundary is one sentence: the line works only on seeds
  assigned to `fabro`; everyone else's seeds are invisible to it.
- Backlog curation becomes an explicit user+agent ceremony; unassigned seeds
  are invisible to the planner but visible in reports, so nothing rots
  silently — it waits.
- The sentinel pair (fabro-919b/fabro-80cc) is redundant once curation is
  done and can be retired; user-owned seeds (grill sessions, sentinels)
  stay unassigned.
- Forgetting an assignment fails closed: a seed waits in the backlog instead
  of being picked up by surprise.
