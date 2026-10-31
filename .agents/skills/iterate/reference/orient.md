# Orientation protocols (Phase 0)

Load at Phase 0. The one-world invariants, the sibling/roster protocol,
the PR sweep, the ledger and its review-gate arithmetic, rootprint, and
the friction score — with the incidents behind each rule. The evidence
index (`incidents.md`) carries the full stories.

## One world

- Everything works on the LINE branch `denkhaus` in the main checkout.
  Runs execute on the PRODUCTION server `https://mirtuell.net` — every
  line query carries `--server https://mirtuell.net`. The local server
  (127.0.0.1:32276) is for TESTS only. git worktrees never.
- `git pull --ff-only origin denkhaus` BEFORE reading tracker state when
  another machine may have run the line — the tracker view is
  branch-local.
- Never checkout/reset the tree while a background cargo runs (the
  builder reads live sources; a mixed build invalidates the whole run).

## Sibling sessions (the roster protocol)

- Establish the sibling situation FIRST (2026-10-04 incident: a second
  /iterate alive in the same checkout found late). Another top-level
  session in this repo's cwd means a second /iterate is alive: the
  uncommitted pool, the tracker file and the ledger are then SHARED,
  and one line has one executor per surface.
- On a sibling: pick a DISJOINT surface (code side vs loop assets),
  claim it in the tracker (a sibling's message queue can be full — a
  message then refuses; claim before messaging). Never commit the
  sibling's files: foreign uncommitted files in `git status` are the
  sibling's work, not an interrupted cycle of this session.
- NEVER `git stash` in the shared checkout (2026-10-17 incident): the
  stash stack is GLOBAL across concurrent sessions — a `-u` stash grabs
  every session's WIP and a pop restores someone else's, swapping the
  dirty-set in one second and misattributing ownership. Tree-state
  probes run read-only; suspending or restoring foreign WIP is never
  this session's operation.
- A checkout still showing `gitbutler/workspace` under
  `git rev-parse --abbrev-ref HEAD` is a machine that has not torn down
  the GitButler experiment — run the teardown ceremony FIRST, then read
  state (both machines tore down 2026-10-16/17; see `incidents.md`).

## Open-PR sweep

- `gh pr list --repo denkhaus/fabro --state open` plus
  `gh pr checks <n>` for each — EVERY gh call carries
  `--repo denkhaus/fabro` explicitly: gh's DEFAULT repo in this checkout
  resolves to fabro-sh/fabro (verified 2026-10-17), a different repo
  with colliding PR numbers; a bare `gh pr merge 427` there hit an
  unrelated 2026-05 merge (no damage, but the checks/greens read first
  were the WRONG PR's).
- A run PR (`fabro/run/*` branches) with a RED dogfood-gate blocks
  auto-merge, fakes in-flight state, and wedges the push gate — repair
  BEFORE selecting: snapshot drift -> accept the snapshot ON THE RUN
  BRANCH and push (auto-merge then lands it); deeper breakage ->
  salvage-or-close decision.
- A PR merged on another machine means the local view is stale: pull
  again. Green-but-waiting PRs: note them, do not touch.

## Line state

- `fabro ps` (mirtuell): a running pass plus an empty ready queue is
  fine (fail-closed park); a PARKED pass with a non-empty queue, a
  PARALLEL pass, or a lost `on_overlap:skip` is an incident. While the
  workflow works the tracker, this session claims nothing — one line,
  one executor.

## Sprint ledger and the review gates

- The ledger is `iterate-state.json` at the repo ROOT (user directive
  2026-10-02 evening: root, never `.fabro/`), session-LOCAL: the LINE
  implements none of the sprint mechanics (no ledger writes in lane
  assets, no reflection parks, no counters — fabro-cadd rejected, PRs
  #359/#360 closed not-planned).
- Reflection invariant: if `sprints_reflected < sprints_completed`, the
  pending reflection runs FIRST (Phase 6 short form) — no new sprint
  starts unreflected.
- Three review gates ride the same cadence shape (user directives
  2026-10-02 / 2026-10-26 / 2026-10-29), each with a marker in
  `iterate-state.json` beside the notes; `null` means never run:
  - CODE-REVIEW (`last_code_review_at_sprint`): due when
    `sprints_completed % 2 == 0` and the marker differs — run the
    code-review skill (two axes, parallel sub-agents, frozen diff,
    tracker specs) over the commits since the last code review.
    Fixpoint = the tip AT THE END of the sprint named in the marker —
    derive it with `git log`, never HEAD~N.
  - ARCHITECTURE (`last_arch_review_at_sprint`): due when
    `sprints_completed % 3 == 0` and the marker differs — fire the
    LOCAL improve-codebase-architecture agent (user directive
    2026-10-02: the architect WORKFLOW stays deactivated on the line;
    never the api trigger). Its top recommendations become work items
    automatically, worked LOCALLY.
  - RUST-SECURITY (`last_rust_security_review_at_sprint`): due when
    `sprints_completed % 5 == 0` and the marker differs — run the
    rust-security skill over the workspace diff since its last run.
  - Setting a marker records only a PASS that actually ran — a deferred
    gate (with rationale) leaves the marker where it was.
- Tracker reads always carry an explicit `--limit` (the facade's list
  caps silently at 50; the retired seeds CLI behaved the same — the
  caution carries over). `work list` for candidates; `work show <id>`
  for ids.

## Rootprint

- Production logs are queried through the `rootprint` CLI, per the
  skill at `~/.prime/agent/skills/rootprint/SKILL.md` (command map:
  search, hist, fields, values, trace, health;
  `search -q 'severity_text:ERROR'` is a valid field query — the
  `level` field does not exist). Do NOT read
  `docker logs rootprint-vector`: that container's stdout carries only
  vector's own boot lines; events go to the opentelemetry sink.
- `severity_text` values are unreliable for catch-alls — filter on body
  text for those. INFO is not ingested, so run timelines come from
  `fabro ps`/events.
- For EVERY run under evaluation: correlate the run's rootprint window
  with its journal painpoints and land findings in ONE work item
  carrying both halves.

## Friction score

- `nu .fabro/scripts/friction-score.nu` once per cycle. Verdicts:
  `normal` (<0.30), `grind` (0.30–0.60), `architecture-due` (>=0.60) —
  they gate Phase 4. A cycle ending in grind journals the components.

## Sprint counting and notes rotation (Phase 6 mechanics)

- A SPRINT is one batch of 2–3 work items (Phase 1 composition) and
  counts ONCE when it closes at least one item this session worked
  LOCALLY with a substantive diff — the batch's other closures ride the
  same sprint. Docs-only, row-only, and verify-only closures never
  increment. LINE closures (loop lane, delegated develop runs) NEVER
  count — the line is not inside a sprint. Historical note: the
  bootstrap-era count of 3 includes develop-lane closures under the
  superseded lane-blind rule; kept as history, not recounted.
- Notes live in the `sprints` array (user directive 2026-10-26):
  records of `{"sprint": <n>, "notes": ["<text>", …]}` — ONE record per
  sprint, SEVERAL notes per sprint possible (corrections and gate
  passes ride alongside), further fields added per need.
- Rotation (2026-10-17 retro, the brightctl port): after appending the
  sprint note, run `just rotate-notes` when sprints > 10 — the state
  file keeps the newest 10 SPRINTS; older records move UNCHANGED, one
  record per line, into `iterate-state-archive.jsonl` (history stays
  git-diffable; both ends validated; dedup keys on the sprint number).
  Deterministic and idempotent; `just rotate-notes-dry` previews. The
  script (`scripts/rotate-sprint-notes.nu`) is SESSION-OWNED: never
  registered in qualitygate batteries, never called by lanes, absent
  from the reflection query — it changes the file, not the loop. 90
  notes once cost every session start the 50 KB read limit (134 KB
  file) — that is why rotation exists.
