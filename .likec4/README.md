# Fabro architecture landscape (LikeC4)

Explorable, tree-structured diagram landscape of the engine. The model
is **target-state first**: it describes the architecture the accepted
ADRs define (ADR-0039 adjudicator–merger at its heart), and marks with
tags where the running system still differs.

## Tooling

The LikeC4 CLI is a devDependency of this package (pinned in
`package.json`, installed with `bun install`):

```bash
bun install
bun run dev              # live preview (port 5173)
bun run validate         # syntax + semantics check
bun run build            # static site into dist/
```

If the dev server exits with `Cannot find package '@tanstack/ai-openrouter'`,
its optional AI assistant picked up `OPENROUTER_API_KEY` from the
environment — start it without the key:

```bash
env -u OPENROUTER_API_KEY bun run dev
```

In the preview, every element with a scoped view is clickable — the
navigation tree mirrors the element hierarchy.

## Layout

| Path                   | Content                                                |
| ---------------------- | ------------------------------------------------------ |
| `spec.c4`              | Vocabulary: element/relationship kinds, status tags. Change rarely. |
| `model/context.c4`     | Level 1: Fabro + external actors (operator, GitHub, Slack). |
| `model/orchestration.c4` | Level 2 control plane: the three workflows, their wiring (all relationships with full FQNs). |
| `model/records.c4`     | Level 2 data plane: run record, observation records, work ledger. |
| `model/run-substrate.c4` | Level 2 execution plane: Petri engine, sandboxes, per-run channel. |
| `views/base.c4`        | Reusable view fragments (`global predicateGroup`).     |
| `views/*.c4`           | One topic per file; `views "Folder"` groups the explorer. |

Cross-file references need full FQNs (`fabro.records.run-record`) —
lexical scope does not survive the file boundary.

## Single-source rules (reuse)

An architecture change must be made in ONE place, never per diagram:

- **Elements are defined once** (extend pattern across files) and only
  referenced by FQN. A rename or description change is one edit.
- **Relationships attach to the owning element** — the most specific
  one (a workflow stage, not the workflow). A view showing a collapsed
  parent aggregates its descendants' relationships automatically; there
  are no duplicate aggregate edges.
- **Shared view fragments are global predicate groups**
  (`views/base.c4`): `orchestration-plane`, `dataset-substrate`,
  `external-forge`, `target-state`. Views compose them with
  `global predicate <name>` instead of copying include/exclude lists.
- **Shared styling is the `status` styleGroup** (spec.c4).

Change scenarios: a new dataset lands in `model/records.c4` and appears
in every `dataset-substrate` view; a workflow rename is one element
title; a retiring authority lands and its `#retiring` edge is deleted
once.

## Status vocabulary

The ADR-0039 restructuring is encoded as tags so diagrams can show
target and today honestly:

- `#planned` — not built (substrate may have landed). Rendered dashed
  and muted.
- `#evolving` — live in predecessor form, rework pending (the merger).
- `#retiring` — an edge or authority ADR-0039 removes (closeout closes,
  merger closes). Target views exclude these edges; the `today` view
  shows them.

When a component lands, flip its tag / drop the retiring edge in the
same change. Elements carry `metadata.code` pointers into the
implementation and `link` to their ADR.

## Refinement path

Each level is a scoped `view X of <element>` — add the view, and the
element becomes clickable in its parent view:

1. **Develop pipeline** — `view of fabro.orchestration.develop`:
   planner → … → reporter stages, gates, PR pipeline.
2. **Observation chain** — closure radius: item ← observation ← run.
3. **Run substrate** — Petri internals (sans-IO core, driver,
   executors), sandbox facets, run-channel transport.
4. **Deployment view** — gears/workflows mapped onto fabro-server,
   SQLite, Docker (logical model stays as is).
5. **Tracker detail** — claim lifecycle, outbox, TrackerProvider.

The `gears` view is the anchor of the landscape: it shows the central
ADR-0039 insight — the gears never talk to each other; the run record's
orchestration fields are the only coupling.
