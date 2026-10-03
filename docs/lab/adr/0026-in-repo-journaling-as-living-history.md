# ADR-0026: In-repo journaling is the living history — growth accepted

Date: 2026-10-03
Status: accepted (user decision 2026-10-03, closing fabro-f97f)

## Context

Every line pass appends run-scoped artifacts to the product tree:
`.fabro/journal/` (one JSONL per run, stage-journal hook), `.fabro/reviews/`
and `.fabro/revisions/` (revisor passes — the revision markers ARE the
revisor's freshness baseline), `.fabro/reports/`, `.fabro/architecture/`.
633 tracked files accumulated in ~4 weeks of line operation (sprint-6 arch
scan). The server-side run store also holds run state, but it is not a
permanent archive.

## Decision

We keep tracking ALL run artifacts in the repo and accept the growth.

- The journaling IS the living history of each repo: durable, greppable,
  citable — independent of any server's retention.
- The server is NOT relied upon to keep runs forever; the in-repo journal
  is the long-term record of record.
- Evidence paths that grep `.fabro/journal/` + `.fabro/revisions/`
  (iterate skill Phase 3, closure verification, salvage) are PERMANENT.
- If repository size ever becomes a real problem, we react then — with a
  deliberate, separate decision (e.g. selective archival). No preemptive
  bounding, no ignore rules, no consolidation.

## Consequences

- Repo growth is linear with line activity; PR diffs interleave product
  changes and the pass's artifacts. That noise is accepted as the price of
  an always-local history.
- The one-file-per-run journal convention stays load-bearing.
- No changes to hooks, workflows, or the iterate skill's evidence paths.
