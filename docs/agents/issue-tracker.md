# Issue tracker: Seeds (sd)

Issues for this repo live in Seeds — git-native issue tracking in `.seeds/`,
driven by the `seeds` CLI. Not GitHub Issues: upstream PRs exist, but tracked
work lives in seeds.

## Conventions

- **Create an issue**: `seeds create --title "..." --type task|bug|feature|epic --priority <1-3>` (1=High, 2=Medium, 3=Low). Labels via `--label`, repeatable.
- **IDs**: short handles like `fabro-6a78`; use them in commits, PR bodies, and docs.
- **Read an issue**: `seeds show <id>` (full body incl. status, labels, evidence).
- **List**: `seeds list` (filters: `--status open|in_progress|closed`, `--type`, `--assignee`); `seeds ready` = unblocked work; `seeds search <query>` = full text.
- **Claim work**: `seeds update <id> --status in_progress` (optionally `--assignee`).
- **Record findings**: edit the seed body (`seeds update <id> --description ...`); there are no separate comment threads.
- **Dependencies**: `seeds dep add <id> <depends-on>`; `seeds ready` respects them.
- **Close**: `seeds close <id>` when done. Before pushing: `seeds sync`.

## Session start

`seeds prime` injects rules and workflow context at the start of a session (see AGENTS.md).

## Assignee: the ownership switch

`assignee` decides who owns a seed — it is the switch that splits work
between the develop line and the user:

- **Filers file UNASSIGNED.** Agents that file seeds (the revisor, any
  agent-originated filing) create them without `--assignee`: filing is
  their job, ownership is not — new seeds land unassigned in the backlog.
- **The develop line only works on seeds assigned to `fabro`.** The planner
  lists candidates with `seeds ready --assignee fabro --limit 200` and claims
  with `seeds update <id> --status in_progress --assignee fabro`; seeds not
  assigned to fabro never appear as candidates.
- **The user can reassign or unassign anytime — that is a veto.** Unassigning
  (or reassigning) a seed removes it from the line's candidate pool on the
  next run; an empty filtered view parks the line rather than falling back
  to unassigned seeds. Assignment is a joint user+agent decision recorded
  in the cycle report (the agent's `--assignee fabro` is a proposal the
  user approves or vetoes by reassigning), never bulk-applied by the
  line itself.

## Upstream PRs

PRs to `fabro-sh/fabro` are referenced from the seed body (e.g. "UPSTREAM PR
#786 OPEN"); the seed closes when the PR merges AND the fix landed on our
branch. PRs themselves are not issues.

## When a skill says "publish to the issue tracker"

`seeds create --title "..." --type <task|bug|feature>` with a body containing
context, acceptance criteria, and evidence links (run IDs, mulch records).

## When a skill says "fetch the relevant ticket"

`seeds show <id>` — the user passes the seed id, or the skill finds it via
`seeds search`.

## Wayfinding operations

Used by `/wayfinder`. The **map** is a seed, **child tickets** are seeds
blocked on it.

- **Map**: an epic-type seed holding Notes / Decisions-so-far / Fog in its description.
- **Child ticket**: `seeds create --type task` + `seeds dep add <child> <map>`; a `wayfinder:<type>` label records the type (research/prototype/grilling/task). Claiming sets `--status in_progress`.
- **Blocking**: `seeds dep add` edges; a ticket is unblocked when every blocker is `closed`.
- **Frontier**: `seeds ready` scoped to the map's children (open, unblocked, not in_progress); first in creation order wins.
- **Claim**: `seeds update <id> --status in_progress`, the session's first write.
- **Resolve**: append the answer to the seed body, `seeds close <id>`, then append a context pointer to the map's Decisions-so-far.
