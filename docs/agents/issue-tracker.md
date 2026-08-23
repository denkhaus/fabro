# Issue tracker: Seeds (sd)

Issues for this repo live in Seeds — git-native issue tracking in `.seeds/`,
driven by the `sd` CLI. Not GitHub Issues: upstream PRs exist, but tracked
work lives in seeds.

## Conventions

- **Create an issue**: `sd create --title "..." --type task|bug|feature|epic --priority <1-3>` (1=High, 2=Medium, 3=Low). Labels via `--label`, repeatable.
- **IDs**: short handles like `fabro-6a78`; use them in commits, PR bodies, and docs.
- **Read an issue**: `sd show <id>` (full body incl. status, labels, evidence).
- **List**: `sd list` (filters: `--status open|in_progress|closed`, `--type`, `--assignee`); `sd ready` = unblocked work; `sd search <query>` = full text.
- **Claim work**: `sd update <id> --status in_progress` (optionally `--assignee`).
- **Record findings**: edit the seed body (`sd update <id> --description ...`); there are no separate comment threads.
- **Dependencies**: `sd dep add <id> <depends-on>`; `sd ready` respects them.
- **Close**: `sd close <id>` when done. Before pushing: `sd sync`.

## Session start

`sd prime` injects rules and workflow context at the start of a session (see AGENTS.md).

## Upstream PRs

PRs to `fabro-sh/fabro` are referenced from the seed body (e.g. "UPSTREAM PR
#786 OPEN"); the seed closes when the PR merges AND the fix landed on our
branch. PRs themselves are not issues.

## When a skill says "publish to the issue tracker"

`sd create --title "..." --type <task|bug|feature>` with a body containing
context, acceptance criteria, and evidence links (run IDs, mulch records).

## When a skill says "fetch the relevant ticket"

`sd show <id>` — the user passes the seed id, or the skill finds it via
`sd search`.

## Wayfinding operations

Used by `/wayfinder`. The **map** is a seed, **child tickets** are seeds
blocked on it.

- **Map**: an epic-type seed holding Notes / Decisions-so-far / Fog in its description.
- **Child ticket**: `sd create --type task` + `sd dep add <child> <map>`; a `wayfinder:<type>` label records the type (research/prototype/grilling/task). Claiming sets `--status in_progress`.
- **Blocking**: `sd dep add` edges; a ticket is unblocked when every blocker is `closed`.
- **Frontier**: `sd ready` scoped to the map's children (open, unblocked, not in_progress); first in creation order wins.
- **Claim**: `sd update <id> --status in_progress`, the session's first write.
- **Resolve**: append the answer to the seed body, `sd close <id>`, then append a context pointer to the map's Decisions-so-far.
