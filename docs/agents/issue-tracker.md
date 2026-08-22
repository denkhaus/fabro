# Issue tracker: Seeds

Issues and specs for this repo live in seeds, a git-native issue tracker. All
operations use the `sd` CLI; data lives as JSONL under `.seeds/` (append-friendly,
`merge=union` merge rules via `.gitattributes`).

## Conventions

- **Create an issue**: `sd create --title "..." --description "..." --type task|bug|feature|epic --priority 0-4 --labels a,b`. Use `--desc` as alias for long bodies. IDs look like `fabro-3a8e`.
- **Read an issue**: `sd show <id>` (or `--format json` for structured data).
- **List issues**: `sd list [--status open|in_progress|closed] [--label x] [--label-any x,y] [--all] [--format json]`.
- **Ready queue**: `sd ready` lists open issues with no unresolved blockers.
- **Update / comment**: `sd update <id> --status ... --title ... --description ...`. There is no separate comment stream; the description is the spec surface — rewrite it, don't append chatter.
- **Apply / remove labels**: `sd label add <id> <label>` / `sd label remove <id> <label>` (also via `sd update --add-label` / `--remove-label`).
- **Dependencies**: `sd block <id> --by <blocker-id>` adds a native blocker edge; `sd unblock <id>` removes one; `sd blocked` lists all blocked issues.
- **Close**: `sd close <id>` (closes via status).
- **Commit tracker changes**: `sd sync` stages and commits `.seeds/` changes.
- **Health**: `sd doctor`.

Infer nothing from remotes; seeds lives in this working copy.

## Pull requests as a triage surface

**PRs as a request surface: no.**

## When a skill says "publish to the issue tracker"

Create a seeds issue with `sd create --title "..." --desc "..."`.

## When a skill says "fetch the relevant ticket"

Run `sd show <id>`.

## Wayfinding operations

Used by `/wayfinder`. The **map** is a single seed with **child** seeds as tickets.

- **Map**: one `--type epic` seed holding the Notes / Decisions-so-far / Fog body in its description, labelled `wayfinder:map`.
- **Child ticket**: `sd create` with body top line `Part of <map-id>`; label `wayfinder:<type>` (`research`/`prototype`/`grilling`/`task`). Once claimed, `--assignee` is set to the driving dev.
- **Blocking**: `sd block <child> --by <blocker-id>` — native edges, the canonical representation. A ticket is unblocked when every blocker is closed.
- **Frontier query**: `sd ready` (open, zero open blockers), drop any with an assignee; first by priority wins.
- **Claim**: `sd update <id> --status in_progress --assignee <name>`, the session's first write.
- **Resolve**: write the answer into the issue description, `sd update <id> --status closed`, then append a context pointer (gist + link) to the map's Decisions-so-far in the epic's description.
