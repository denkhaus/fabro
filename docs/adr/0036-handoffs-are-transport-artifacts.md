# ADR-0036: Handoffs are transport artifacts, not history

- Status: Accepted (2026-10-26, user directive during the sprint-54
  architecture-gate pass; superseded-banner approach rejected by the
  user in the same breath)
- Deciders: user (directive 2026-10-26, paraphrased: handoffs exist only
  to transport tasks between sessions; once the tasks are worked,
  markdown handoffs are deleted and work-item handoffs closed — banners
  rejected in the same breath), agent (execution, reference cleanup)
- Relates to: ADR-0032 (work items are the durable tracker — the
  sprint order lives there, not in handoff prose), ADR-0033 (written
  language rule — this ADR records why handoff prose is not kept at
  all)

## Context

Handoff documents (markdown situation reports under `docs/internal/`)
carried tasks from one session to the next across session boundaries.
Two accumulated: `line-restore-handoff.md` (2026-10-20) and
`work-tracker-handoff.md` (2026-10-21). Both had long since been
consumed — their demands landed — yet both stayed in the tree. The
sprint-54 architecture scan flagged the newer one: a session entering
through `docs/internal/` reads a stale situation report ("Phase 2
blocked", "rewrite has NOT started") as if it were current, next to the
authoritative migration doc.

The prior convention was superseded banners: keep the file, prepend a
note pointing at the successor. The 2026-10-20 handoff had received
one; the scan proposed giving the 2026-10-21 handoff the same.

## Decision

A handoff is a **transport artifact**. It exists only to carry tasks
from one session to the next; it is not a record. The moment its
demands are consumed (landed, or filed as work items), the consuming
session destroys it:

- A **markdown handoff file** is `git rm`'d in the consuming session.
  Git history is the archive; no superseded banners, no doc graveyard.
- A **work-item handoff** is closed with the evidence as the close
  reason (the facade's close comment).

The consuming session cleans references to a deleted handoff
(checklists, docs) in the same change. **ADR citations stay
untouched** — an ADR's "Revises:" line is a historical record and keeps
naming files that no longer exist in the tree.

## Alternatives rejected

- **Superseded banners** (the prior convention): keeps a graveyard of
  stale situation reports that reproduce exactly what `git log` already
  preserves; every future session pays the read-and-distrust cost.
- **Archival subfolder** (`docs/internal/archive/`): the same retention
  with extra path churn; "archived" markdown is still findable and
  still misleading.

## Consequences

- `docs/internal/` holds only living documents; a session reading it
  sees current state only.
- The transport function is unchanged while active: a handoff file may
  exist and be read while its tasks are open — deletion binds at
  consumption, not at creation.
- Leaving a consumed handoff in the tree is a review finding (same
  class as stale docs generally).
- The iterate skill carries the process pointer to this ADR; the
  2026-10-26 deletion of both named handoffs is the first application
  (commit message cites this ADR's number).
