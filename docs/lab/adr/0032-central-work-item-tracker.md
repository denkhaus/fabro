# ADR-0032: Central private work-item tracker with a server-owned claim ledger

- Status: accepted (user decision, grill-with-docs session, 2026-10-20)
- Deciders: user + agent
- Related: ADR-0017 (engine stays tool-agnostic), ADR-0018 (seed ownership
  and assignment boundary), ADR-0019 (capability gate for agent surfaces),
  ADR-0023 (native seeds product), ADR-0031 (run-PR merger), fabro-b8b7,
  fabro-a33e, fabro-997d, fabro-9372, fabro-4580,
  `docs/internal/central-work-tracker-migration.md`
- Revises: the seed-as-coordination-store assumption of the existing line
  workflows and the Phase-2 build form in `docs/internal/line-restore-handoff.md`
  (the handoff's "assignment is the broken half" diagnosis stands; the
  proposed store does not).

## Context

On 2026-10-20 one 40-minute window produced three duplicate
implementations/re-closures across loop and develop (#449 vs #448 on
fabro-16a7; #455 vs #453 on fabro-7f58; #457 re-closed fabro-bdff 30
minutes after #453), two duplicate revisor passes on the same runs, and
zero revisor-filed seeds (every finding stranded as overflow in revision
markdown). One duplicate run was reviewed as healthy because the revisor's
dup-run-check read its frozen tracker as OPEN.

The ADR-0031 run-PR merger worked throughout: landing is deterministic and
ordered. The broken half is **assignment**: every run claims from a frozen
clone, writes the claim to its own `.seeds/issues.jsonl` copy, and nothing
server-side arbitrates. The claim becomes visible only when the run PR
lands, minutes to hours later.

The file tracker is used as a replicated database and pays the price:

- 938 items, 2,059,279 bytes of JSONL, 1,302 commits touching
  `.seeds/issues.jsonl` since 2026-09-01.
- `seeds doctor` reports 17 bidirectional dependency mismatches and 52
  inconsistent status/closedAt pairs; two items sit in `in_progress`
  without an assignee.
- IDs are 4 hex characters (65,536 values). At 938 items the expected
  number of collision pairs under a uniform generator is ~6.7; `seeds
  dedupe` and a "0 duplicates" handoff check exist because collisions are
  structural, not exceptional.
- Claim, close and filing are file writes in a run branch. Revisor
  findings need a PR to reach the tracker; in the incident window none
  did.
- The tracker is already public: `denkhaus/fabro` is public and the
  `denkhaus` branch carries `.seeds/issues.jsonl` (raw URL readable
  without login). The current setup does not satisfy a confidentiality
  requirement.

Fabro's own line is one deployment, not the product. Fabro must work for
projects that use a different tracker repo, a per-repo tracker, an
external tracker, or no tracker at all. Any design that hard-codes
`denkhaus/work`, `project:fabro`, `fabro-xxxx`, `seeds`, or GitHub labels
into the engine or the workflow prompts is rejected.

## Decision

### 1. Separate the two data planes

- **Work item** (what/why): title, body, labels, priority, dependencies,
  links. Durable, human-visible, reviewable. Owned by a tracker provider.
- **Claim/queue** (who/when): the assignment of one item to one run,
  in-flight state, pipeline position, release. High-frequency,
  conflict-prone, needs transactional arbitration. Owned by the server.
- **Pipeline** (run -> PR -> merge): already owned by the server (run
  projection, pull_request projection, ADR-0031 merger).

The file tracker conflates all three. The new design never does.

### 2. The server is the assignment authority

A `ClaimLedger` in the server owns grants, uniqueness, release and
recovery. It is project-scoped and tracker-independent:

```
projects(
  project_id, tracker_provider, tracker_repo, project_label,
  lane_labels, claim_label_prefix, claim_comment_enabled
)
claims(
  claim_id, project_id, item_key, run_id, lane, state,
  granted_at, released_at, release_reason
)
claim_events(claim_id, at, from_state, to_state, reason, run_id)
outbox(id, kind, payload, idempotency_key, attempts, next_attempt_at)
```

Partial unique indexes enforce:

- one non-terminal claim per `(project_id, item_key)`;
- one non-terminal claim per `run_id`.

Non-terminal means `state IN ('granted','active','awaiting_merge')`.
`awaiting_merge` still holds the item: a pending PR is not available for a
new run.

State machine:

```
granted -> active -> awaiting_merge -> landed
granted | active | awaiting_merge -> released
```

- `granted`: DB transaction committed; projection may still be pending.
- `active`: run is non-terminal; the claim label is present.
- `awaiting_merge`: run is terminal and its PR is open; the label is
  removed; the issue stays open and unclaimable.
- `landed`: PR merged; the issue is closed.
- `released`: run failed/cancelled, PR closed without merge, duplicate
  close, external close, or orphan; the issue is available again.

Grant order is **DB first, projection second**:

1. insert/verify the claim in a transaction and commit;
2. append a `work.claimed` platform record to the run stream;
3. enqueue the tracker projection through the outbox.

A crash before step 1 loses nothing. A crash after step 1 leaves a valid
claim whose projection is healed by the reconciler. A GitHub outage never
blocks a grant; the outbox retries.

Backpressure is server-enforced: a lane refuses a new grant while its run
PRs are not drained (fabro-4580). A scheduler that forgets the guard
cannot create a second in-flight run for the lane.

### 3. The tracker is a provider, not a hard dependency

A `TrackerProvider` contract covers `list`, `get`, `create`, `close`,
`comment`, `project_claim` and `clear_claim`. The server resolves the
provider and its configuration from a server-side `ProjectRegistry`; the
run declares a project id (explicit config, or derived from its run
target). The API and the UI are project-scoped from day one:
`/api/v1/projects/{id}/work-items`, `/api/v1/projects/{id}/claims`,
`/api/v1/projects/{id}/queue`.

First adapters:

- `GitHubTracker` — issues in a configured repository, labels mapped from
  the project config;
- `SeedsTracker` — read-only for migration and for projects that still use
  `.seeds`; never the line's write path;
- `NullTracker` — manual/no tracker; the claim ledger still works.

The core knows no tracker vendor, no repository name, no label spelling.

### 4. The first provider is one central private GitHub issue repository

For the user's deployments, all work items of all projects live in **one
private GitHub repository**, `denkhaus/work` (deployment policy, not a
core invariant).
There are no per-repo issue tabs and no per-repo companion repositories.
The repository is configured per project; the same central repo can serve
many projects through `project:<id>` labels. A project that prefers its
own repo or another provider changes config, not code.

Issue content is the durable spec. Labels carry the queue classification:
`project:<id>`, `lane:<lane>`, `type:<type>`, `prio:<p0..p4>`,
`source:<user|revisor|architect|agent>`, plus `needs-user`, `revision`,
`sensitive`, `blocked`. Claim state is not a label and not a comment; it
is the server ledger plus the run-id projection below.

GitHub's closing keywords are not relied on: the line's PRs target
`denkhaus`, not the repository default branch, so `Closes` does not
auto-fire. The ADR-0031 merger closes the issue through the API after a
merge (cross-repo `Fixes <tracker>#<n>` is written into the PR body for
the human cross-reference). A duplicate close releases the claim and
leaves the issue open.

### 5. The claim projection is the run-id label `fabro:run:<id>`

A boolean `state:claimed` label is not reconstructable: it carries no run,
lane, timestamp or release evidence. The run id does. While a claim is
`active`, the issue carries one label:

```
fabro:run:<run_id>
```

The prefix is configurable (`claim_label_prefix`); the default namespaces
the shared tracker. The label is a **pointer**, not the record. Lane,
project, status, PR and timestamps are derived from the run metadata
(`current_work_item` in the run projection, fabro-9372 extended to the
work item key). Grant time and release reason live in the claim record;
the issue timeline records label/unlabel events; the run stream records
`work.claimed`/`work.released`.

Label mechanics:

- GitHub requires a label object to exist before it can be attached to an
  issue (422 otherwise). The adapter creates it first (`POST /labels`,
  treating "already exists" as success), then adds it to the issue.
- On `awaiting_merge` or `released`, the label is removed; the label
  object is deleted; a periodic GC removes `fabro:run:*` labels with no
  issues.
- One active claim per run and per item is enforced by the DB, not by the
  label. Epics are linked, not claimed.
- A human who removes the label while the run is active is corrected by
  the reconciler; a human who adds a bogus run label is corrected the same
  way.

### 6. Durability and recovery

- **Process crash:** SQLite WAL keeps the claim. Startup reconciliation
  re-projects it.
- **GitHub outage:** the DB and the line continue; outbox entries retry.
- **DB loss:** the tracker is the external index. Scan open issues for
  `fabro:run:*`, extract run ids, adopt claims whose run is live, release
  the rest. If the run store is also lost, release all claims and let the
  line re-claim; open run PRs and sandbox labels (`petri.run`) are the
  second source.
- **Orphan claim:** a claim whose run record is missing or whose run is
  terminal without a PR is released after a short grace period (30
  minutes), not after the old 6-hour tracker guard.
- **Backup:** the claim tables are small; a daily SQLite snapshot is the
  primary recovery. The tracker label/comment timeline is the external
  recovery key.

The old `tracker-guard.nu` 6-hour stale-claim arm retires with the file
tracker. Claim validity follows run status, not wall-clock age.

### 7. Agent surface stays engine-mediated

New run tools, approved by this ADR under ADR-0019:

- `fabro_work_next` — atomically grant the next eligible item for the
  run's project and lane; returns the item content;
- `fabro_work_show` — read one item;
- `fabro_work_create` — file an item (revisor/architect; no claim);
- `fabro_work_close` — close an item with a reason;
- `fabro_work_release` — release a claim the planner will not use
  (misroute, stale basis).

Writes are executed by the server against the provider. Agents get no raw
GitHub token and no `gh` write path (ADR-0019). Host sessions may use
`gh` with the user's own auth; runs do not.

The planner's grant is tool-based, not a pre-LLM preflight, because the
planner owns surface routing and must be able to release a misrouted item
and ask for the next one without a new run. The deterministic preflight
may still pre-compute candidates, but the atomic grant is the tool call.

### 8. Merger and PR pipeline are unchanged

ADR-0031 stands. The merger remains the only landing path. It closes the
linked issue on merge, releases the claim on a duplicate close, and feeds
the queue view. GitHub's merge queue is not adopted: it has no policy
windows, no duplicate-file close, and no base-first ordering.

The server exposes one pipeline view per project and lane: claimed,
active, awaiting merge, landed, released. The web UI and the optional
GitHub Projects board read that view; the board is presentation, never
authority.

### 9. Migration and secrecy

- All items migrate to the central private repository. Open items first;
  closed items are imported as closed with `Original-Created` /
  `Original-Closed` in the body (GitHub cannot set historical timestamps).
  The JSONL is archived in the tracker repository. Legacy ids are kept as
  `Legacy-ID: fabro-xxxx` in the body; new items use the issue number.
- The line's `.seeds` writes stop at cutover. The file is frozen; the
  `seeds` CLI leaves the line's toolchain and prompts. The seeds product
  (ADR-0023) remains a product and a provider; it is no longer the line's
  default tracker.
- The tracker repository is private. The code repository may stay public.
  A `sensitive` item gets a neutral public run/PR/commit title; its
  details live only in the private issue. Vulnerabilities use GitHub
  Security Advisories, not the tracker, for exploit details.
- The existing `.seeds` history is already public. This ADR removes the
  file going forward; it does not rewrite git history. A history rewrite
  is a separate, explicit decision.

## Rejected alternatives

- **Seeds plus a server claim ledger (the handoff's Phase 2 as written).**
  It fixes assignment but leaves the file as a replicated database:
  JSONL merge conflicts, dedupe/doctor, 4-hex ids, frozen-clone content
  edits and filing via PR remain. It is a bridge, not an end state.
- **Per-repo GitHub issues (same repo as the code).** Privacy would switch
  with repo visibility; the user rejected a rule that changes by repo
  type. One central tracker is simpler and gives one board.
- **One companion repository per code repository.** Not practical; N
  repositories for N projects, and every project pays the setup cost.
- **GitHub as the assignment authority without the server.** GitHub has no
  compare-and-swap on assignees, labels or fields; two simultaneous claims
  both succeed. The server mutex is required in every variant.
- **A `state:claimed` boolean label.** Not reconstructable: no run id, no
  lane, no timestamp, no release evidence.
- **A claim comment instead of the run-id label.** More duplication; the
  run metadata already carries the rest. The label is the better pointer,
  and the issue timeline records its add/remove events.
- **Building a server-native tracker now.** It is the right endgame if
  the tracker becomes a product surface, but it means building UI, search
  and notifications that GitHub provides. The provider interface keeps
  that option open.
- **Relying on `Closes #n`.** The line merges to `denkhaus`, not the
  default branch; auto-close is not reliable. The merger closes issues.
- **Replacing the merger with GitHub's merge queue.** No windows, no
  duplicate-file close, no base-first ordering.

## Consequences

- Run PRs no longer carry tracker churn; the JSONL merge path and the
  closed-wins union for tracker-only conflicts disappear from the merger.
- The code repository's history stops accumulating one tracker commit per
  run (1,302 since 2026-09-01).
- Revisor and architect findings reach the tracker immediately; the
  overflow-stranding class closes. The ADR-0022 filing balance remains a
  policy and can be enforced server-side.
- The line depends on the tracker repository and the GitHub API. The
  outbox, the local claim ledger and the startup reconciler bound the
  blast radius; a GitHub outage does not stop active runs.
- New agent tools and a project-scoped API/UI are capability and product
  additions, approved here for the work-item surface and gated by
  ADR-0019 for anything beyond it.
- Cross-repo issue links are less integrated than same-repo links; the
  server writes the PR link to the issue and the issue reference to the PR.
- The product gains a project-scoped work-item/claim surface. Fabro's own
  line is one deployment; the provider registry is the product seam.
- Migration is a multi-phase project (server core, adapter, workflow
  rewrite, data migration, cutover). The line stays off until the
  verification matrix in the migration plan passes.

## Follow-ups

- Verify GitHub App permissions for issue writes and Projects v2 on the
  chosen owner (personal vs organization); extend the manifest if a board
  is wanted.
- Extend fabro-9372 from `current_seed_id` to `current_work_item`
  (project + item key).
- Decide whether the ADR-0022 filing balance is enforced server-side and
  whether `overflow-ledger.nu` retires.
- Revisit ADR-0023's route: the seeds product remains, but it is no longer
  the line's default tracker. If that changes the native-seeds roadmap,
  record it in a follow-up ADR.
- The migration plan is `docs/internal/central-work-tracker-migration.md`.
