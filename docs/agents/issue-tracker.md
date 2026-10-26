# Issue tracker: Fabro work items (the central tracker, ADR-0032)

Issues for this repo live as **work items** in project `fabro` on the
production server, backed by the private tracker repo `denkhaus/work`
(GitHub Issues is the storage; the facade is the only session path).
Driven by the `fabro work` CLI. `.seeds` is RETIRED: frozen at the
cutover, history archived byte-exact under
`denkhaus/work/archive/denkhaus-fabro/`.

## Vocabulary (the word "seed" is retired)

An old "seed" is now a **work item**. Say "file a work item", never
"create a seed" — no new item carries a seed identity; only the 974
migrated items keep a `seeds:fabro-xxxx` label as their legacy anchor
(and old references keep resolving through it).

- "Erstelle ein Work-Item zu X" → create WITHOUT a lane: the item lands
  in the human-triage backlog.
- "Erstelle es auf der develop-Lane" → create with `--lane develop`: the
  develop line's candidate pool.
- "Schließe fabro-xxxx" → the legacy id still names the item; `show` and
  `close` accept it directly.
- "Was liegt auf develop?" / "Zeig das Backlog" → `list --lane develop` /
  the unlaned open items.

## Conventions

- **Server**: always pass `--server https://mirtuell.net` explicitly —
  never rely on `FABRO_SERVER` (bench hygiene rule). The `work` verb
  ships with the fork.26 build; until that deploy, use a tree build
  (`cargo build -p fabro-cli`, then `./target/debug/fabro work …`).
- **Create**: `echo "$body" | fabro work --server … create --project fabro
  --title "…" --body-file - [--lane develop] [--type bug] [--priority 1]
  [--label topic]`. Bodies NEVER travel on a command line — `--body-file -`
  reads stdin. `--type`/`--priority` derive the `type:<t>`/`prio:pN` labels
  (never hand-spell them; a contradicting raw `--label` of the same family
  is refused). Title and body must be non-empty; the server stamps
  `project:fabro` itself.
- **Keys**: the item key `denkhaus/work#N` is the canonical handle (use
  it in commits, PR bodies, docs). A bare `N` composes against the
  project's trackerRepo; a legacy id `fabro-xxxx` resolves through the
  `seeds:` label seam (server side from fork.26).
- **Labels (the standard)**: `prio:p0..p4` (default p2) ·
  `type:task|bug|feature|epic` ·
  `lane:loop|develop|architect|conductor|revisor` (open items only) ·
  free-form topic labels (`revision`, `engine`, …). `seeds:fabro-xxxx`
  is LEGACY-ONLY — never apply it to a new item.
- **Lanes are the ownership switch** (replacing the old `assignee`
  field, which work items do not have): an open item with NO lane = the
  human-triage backlog; `lane:develop` = the develop line's candidate
  pool; `lane:loop` = the loop line. The user moves items between lanes
  or closes them — that is the veto. Agents file unlaned unless told
  otherwise; the line only works its own lane.
- **Read**: `list --project fabro [--lane develop] [--limit N]
  [--query text]` (list is open-only; `--query` is the road to closed
  items) · `show --project fabro <selector>` (head + body verbatim;
  `--json` for structure) · `claims --project fabro` (the live run
  ledger, read-only).
- **Close**: `close --project fabro <selector> --reason "<evidence>"` —
  `--reason` is required and becomes the close comment; that is where
  the evidence lives (run ids, PR, commit).
- **Dependencies**: no native edges — record `Blocked-By: <key, …>` /
  `Blocks: …` lines in the body (the canonical trailer form the import
  established).
- **Upstream PRs** to `fabro-sh/fabro` are referenced from the item
  body; the item closes when the PR merges AND the fix landed on our
  branch. PRs themselves are not issues.

## When a skill says "publish to the issue tracker"

Create via the stdin form above with context, acceptance criteria, and
evidence links (run ids, PR urls) in the body.

## When a skill says "fetch the relevant ticket"

`fabro work --server … show --project fabro <key|number|fabro-xxxx>`;
find candidates with `list --project fabro --query <terms>`.

## Wayfinding operations

No `dep` edges exist; dependencies are body trailers. Map = a
`type:epic` item holding Notes / Decisions-so-far / Fog in its body.
Child ticket = an item whose `Blocked-By:` names the map; unblocked =
every blocker closed (readable via `show`). Frontier = the map's open,
unblocked children, first by number. Resolve = append the answer to the
item body (`update` endpoint), `close`, then append the pointer to the
map's Decisions-so-far.
