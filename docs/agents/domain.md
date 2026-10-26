# Domain Docs

How the engineering skills should consume this repo's domain documentation
when exploring the codebase. Layout: **single-context**.

## Where things live

- `GLOSSARY.md` (repo root) — the canonical glossary. Line and
  work-tracker vocabulary; terms are binding for work items, prompts,
  and ADRs.
- `docs/adr/` — the decision record. One continuing numbered series
  with three eras of provenance (user record, 2026-10-26):
  - **0001–0008, lab/meta era**: rescued from the retired `lab/meta`
    branch; per-world platform decisions (see ADR-0005 for that era's
    own rules).
  - **~0009–0031, lab-branch era**: workflow-authoring decisions,
    developed on the `lab` branch decoupled from the engine on
    `denkhaus`. This era includes the German-titled `rev`/`rev2` files
    (revisor/engine-side revision decisions, e.g.
    `0009-rev-fs-policy-flow-on-petri.md`,
    `0021-rev2-quota-park-on-petri.md`) that SHARE numbers with the
    main series — citations resolve by full filename, and where a
    skill cites a bare "ADR-0009"/"ADR-0021" it means the English
    stage-envelope / line-recovery files.
  - **0032+, denkhaus line**: the line's own decisions, written in this
    tree, English per ADR-0033.
- `docs/lab/CONTEXT.md` — the lab branch's platform glossary
  (legacy; still cited by lab-era ADRs). Root `GLOSSARY.md` is
  canonical; lab terms migrate into it on touch.
- `docs/lab/` (rest) — rescued lab-era material: review reports, the
  engine docs snapshot, petri analyses.

## Before exploring, read these

- `GLOSSARY.md` at the repo root.
- The ADR in `docs/adr/` that owns the area you touch (cited by number
  in skills, work items, and code comments).

If a file you'd expect doesn't exist, proceed silently. Don't flag its
absence; don't suggest creating it upfront. `/domain-modeling` (reached
via `/grill-with-docs` and `/improve-codebase-architecture`) extends
the glossary lazily when terms actually get resolved.

## Use the glossary's vocabulary

When your output names a domain concept (in a work item title, a
refactor proposal, a hypothesis, a test name), use the term as defined
in `GLOSSARY.md`. Don't drift to synonyms the glossary explicitly
avoids. If the concept you need isn't in the glossary yet, that's a
signal: either you're inventing language the project doesn't use
(reconsider) or there's a real gap (note it for `/domain-modeling`).

## Flag ADR conflicts

If your output contradicts an existing ADR, surface it explicitly
rather than silently overriding:

> _Contradicts ADR-0007 (event-sourced orders), but worth reopening because…_

Opening a superseding ADR beats silently widening an accepted one; the
complexity brake (ADR-0028 lesson) applies.
