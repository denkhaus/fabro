# Glossary

The working vocabulary of the work-item tracker (ADR-0032). One term, one
meaning; every entry names where the term lives in code and what it is
confused with.

## Work item

One unit of work a human files and a run eventually closes: a title, a body,
labels, a priority and a state (`open` / `closed`). Content is owned by a
tracker provider (GitHub today), never by the server.

- Key: `item_key`, `<trackerRepo>#<number>` for GitHub — e.g.
  `denkhaus/work#12`. This is what `fabro work show` takes and what the API
  returns as `itemKey`.
- Priority: derived from the `prio:pN` label, default `2`.
- Not a **claim**: an item says what should happen; a claim says who is doing
  it right now. An item outlives every claim on it.

## Claim

The server-owned assignment of one work item to one run, arbitrated in the
claim ledger (ADR-0032 decision 2). High-frequency and conflict-prone, which
is why it never travels through the tracker.

- Unique per `(project, item)` and per run while non-terminal.
- States: `granted` → `active` → `awaiting_merge` → `landed`, plus
  `released` when a run gives the item back.
- Carries the run id, the lane, and the PR url of the run that holds it.
- Not a **label on the item**: a claim is a ledger row. The tracker carries
  at most the visible echo of one (`claimLabelPrefix`, `claimCommentEnabled`).

## Work project

One registered tracker binding: a project id, a provider, a tracker
repository, the label that scopes items to it (`projectLabel`, e.g.
`project:fabro`), the lane→label map, and the per-lane backpressure cap.

- `fabro work` verbs always name it explicitly with `--project <id>`; there
  is no implicit default, so a script line always says which project it
  means.
- Not a **code repository**: `targetRepo` (where the runs work) is an
  optional second field. A project may track work without owning code.

## Lane

A named queue inside a work project (`loop`, `develop`, …). The lane name is
what callers pass (`--lane loop`); the project's `laneLabels` maps it to the
tracker label that marks an item as belonging to that lane
(`lane:loop`). Lane labels are how an item is queued; the lane is also the
unit of backpressure (`backpressureOpenPrCap`).

- Not a **label**: the label is the projection, the lane is the concept. An
  unknown lane name is a hard error, never a silent fallback to a raw label.

## Legacy id

A `.seeds` identifier (`fabro-d5f0`), optionally written `seeds:fabro-d5f0`.
It names an item that lived in `.seeds/issues.jsonl` before the cutover and
now lives on the tracker.

- Accepted by `fabro work show` / `close`, which pass it to the server
  unchanged; the server resolves it through the migration label seam
  (`seeds:<id>` on the tracker item, stage 1a) and returns the item.
- A **transition affordance with a removal deadline**, not a second key: new
  items never get one, and once the migration window closes the seam and the
  acceptance go together. The live proof of that resolution waits for the
  import, which is the step that writes the labels.

## Closure radius

The set of a work item's open CHILDREN. A parent's closure radius is
empty when every painpoint item born from it is closed; the `work
close` verb refuses while the radius is open (an explicit override
names the children). Tranche selection prefers closing open radii
before opening new themes — a preference, never a gate.

## Parent / Child (work item)

One work-item relationship, carried as a GitHub sub-issue link, with
one meaning: the child documents its ORIGIN (the run that worked the
parent produced it) AND gates the parent's closure. The parent of a
painpoint is the run's claimed item; a theme epic may parent manually.
Children found after the parent closed anchor to the next open anchor
— they never reopen a closed parent.
