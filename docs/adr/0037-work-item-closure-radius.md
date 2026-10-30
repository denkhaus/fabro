# ADR-0037: Work-item closure radius — painpoint children gate their parent's close

## Status

Superseded by ADR-0038 (2026-10-30, same session): the closure radius
survives, but its carrier changed — painpoints became platform records
and the chain gained the intermediate painpoint node. The sub-issue
carrier and the direct parent/child link below are REPLACED; the
close-gate, timing, and preference decisions carried over.

Accepted (2026-10-30, grilled design — two rounds, eleven decisions).

## Context

Every run births work items: the lap journals carry painpoints, the
session and gates file follow-ups, the revisor files improvements. The
audit (denkhaus/work#1033) found 372 open items, 279 alive after
vetting — a flat pool that ages silently, and no structural coupling
between a worked theme and the items it spawned. Working themes
sequentially is the user's stated preference, but nothing in the model
expresses "this theme's painpoint radius is still open", so new themes
open while old radii rot.

## Decision

Work items link as PARENT and CHILD through GitHub sub-issues. One
relationship, one meaning: a child documents its ORIGIN (derived-from
the parent whose run produced it) AND gates the parent's closure
(blocks-close). The design in eleven settled points:

1. One unified link (derived-from + blocks-close in a single
   relationship).
2. Carrier: GitHub sub-issues — the tracker stays the single source of
   record; the UI renders the tree.
3. The `work close` verb refuses to close a parent while children are
   open; `--force` overrides with a mandatory reason and the close
   message lists the open children automatically.
4. The parent is the run's claimed item; a theme epic may parent
   manually where a painpoint belongs to the theme, not the item.
5. Timing: the closeout files a run's painpoints as children BEFORE
   closing the parent. Painpoints found after the parent closed
   (revisor, session analysis) anchor to the next open anchor — they
   never reopen a closed parent.
6. Sequencing is a preference, not a gate: tranche selection prefers
   closing open radii before opening new themes; no hard rule, no
   deadlock.
7. The existing pool is not backfilled wholesale; the next relevance
   pass backfills opportunistically where a theme is still open.
8. Preference mechanics: a radius view (open children grouped by
   parent) feeds the hand-picking; the claim picker stays unchanged.
9. Facade: `work link <child> --parent <key>`, `work show` renders the
   tree; the tracker provider gains sub-issue read/write.
10. The closeout links its painpoint filings automatically at filing
    time; the session and revisor link manually.
11. The work is one epic with a decomposition note (link seam, close
    gate + override, closeout auto-link, radius view), each stage
    independently landable.

## Consequences

- A theme's completion becomes checkable: the radius view answers
  "which themes still have open painpoints" in one query.
- The item flood gets structure: new items arrive as children, visible
  under their theme, instead of joining a flat pool.
- Closing a parent requires either finished children or an explicit,
  recorded override that names them — silent theme abandonment becomes
  a visible act.
- The tracker provider and the work facade gain sub-issue surface
  (API, wire pins, facade verbs); the close verb's refusal needs
  error-shape and override pins.
- Post-hoc painpoints attach to the NEXT open anchor, which keeps
  closed themes stable at the cost of some historical precision — a
  deliberate trade (the reopen spiral was rejected).
