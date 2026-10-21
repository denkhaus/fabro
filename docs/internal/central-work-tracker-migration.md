# Migration plan: central work-item tracker and server-owned claims

Status: proposed (direction user-approved 2026-10-20; implementation not started)
Authority: `docs/lab/adr/0032-central-work-item-tracker.md`
Audience: the next host-side session (the line stays OFF until the checklist
passes)
All user-facing times: Europe/Berlin. The document's internal timestamps,
when added during execution, use Europe/Berlin and say so.

## Locked decisions

The grill session settled these; do not re-open them without the user.

1. **Two planes.** Work-item content lives in a tracker; claim/queue state
   lives in the server. The file tracker is not a coordination database.
2. **One central private GitHub issue repository per deployment** for all
   projects. No per-repo issue tabs, no per-repo companion repositories.
3. **The server is the assignment authority.** A transactional claim ledger
   is the mutex; GitHub is the store and the human view.
4. **The claim projection is the run-id label** `fabro:run:<run_id>`, not a
   `state:claimed` boolean and not a claim comment. The label is a pointer;
   run metadata derives lane, project, status, PR and timestamps.
5. **The label is removed when the run ends** (`awaiting_merge` or
   `released`). The issue stays open while the PR is pending; the server
   queue is the pipeline view.
6. **Durability:** DB first, projection second, outbox, startup reconciler,
   rebuild-from-labels. No 6-hour tracker guard; claim validity follows run
   status, with a 30-minute orphan grace.
7. **Provider and project are abstractions.** The core knows no tracker
   vendor, repository name, label spelling or `seeds`/`fabro` identity.
   GitHub is the first full adapter; seeds is a read-only migration adapter;
   the central repo is a deployment policy.
8. **Agent writes are engine-mediated.** `fabro_work_*` run tools; no raw
   `gh` or GitHub token in sandboxes (ADR-0019). Host sessions may use `gh`
   with the user's auth.
9. **The ADR-0031 merger is unchanged.** It closes issues on merge and
   releases claims on duplicate closes. GitHub's merge queue is not used.
10. **Secrecy:** the tracker is private; the code repo may stay public.
    `sensitive` items get neutral public run/PR/commit titles; details stay
    in the private issue; vulnerabilities use Security Advisories.
11. **IDs:** migrated items keep `Legacy-ID: fabro-xxxx` in the body; new
    items use the issue number. Old references resolve through the
    tracker itself, not a mapping file: every migrated issue carries the
    legacy id as a namespaced label (`seeds:fabro-xxxx`), so an exact
    label lookup resolves old ids with no false positives (bodies citing
    other ids do not match). Amended 2026-10-21 (user decision, replacing
    the original `seed-id-map.json` file; label prefix `seeds:` chosen over
    `legacy:` the same day so the prefix names the origin system, not a
    deprecation): the label IS the mapping; the tracker is the single
    source of truth.
12. **Migration scope:** all items, open first, closed imported as closed
    with original dates in the body, JSONL archived in the tracker repo.
13. **No history rewrite** of the already-public `.seeds` file as part of
    this migration; that is a separate decision.

## Target shape

```
Code repo (any visibility)          Central private tracker repo
  run branch / PR  ------------->     issue #123
  PR body: Fixes owner/work#123       labels: project:fabro, lane:loop,
                                               type:bug, prio:p2, ...
                                      label while active: fabro:run:<id>
                                      timeline: label/unlabel events

Fabro server
  ProjectRegistry  -> project id, provider, tracker repo, label mapping
  ClaimLedger      -> claims, claim_events, outbox, unique active claims
  Reconciler       -> DB <-> tracker projection; rebuild from labels
  Queue view       -> claimed | active | awaiting_merge | landed | released
  Run tools        -> fabro_work_next / show / create / close / release
```

## Phase 0 — Record and freeze (before any code)

- [ ] ADR-0032 accepted; this plan reviewed; user GO for implementation.
- [ ] Update the tracker epic fabro-b8b7 with the revised store decision
      and the fork answers (A: server claim table + label projection;
      B: `fabro_work_next` tool; C: run-status release + 30-min orphan).
- [ ] File one migration epic (unassigned; assignment is the user's
      decision, ADR-0018).
- [ ] Add a "superseded" note to `docs/internal/line-restore-handoff.md`
      pointing at ADR-0032 and this plan.
- [ ] Confirm the line is off and `gh pr list --state open` has no run PRs.
- [ ] Use `denkhaus/work` (Q8). Confirm the GitHub App installation on it
      and the Projects permission if a board is in scope. An organization
      move is a later config change, not a Phase-0 blocker.
- [ ] Verify the existing seeds CLI/toolchain references that will be
      retired (`rg -l '\bseeds\b' --glob '!*.jsonl'`; 94 files today).

## Phase 1 — Server core (no GitHub writes yet)

Build in `lib/apps/fabro-server` (OpenAPI-first), `lib/components/fabro-work`
(the forge-agnostic port + store; user direction 2026-10-20) and
`lib/components/fabro-github` (the GitHub adapter).

**Status 2026-10-20 (first vertical slice landed):** the `work_queue` SQL
migration (`work_projects`, `work_claims`, `work_claim_events`,
`work_outbox`), the `fabro-work` crate (store + tracker port + NullTracker),
the `GitHubTracker` adapter, the project-scoped API (`/projects`,
`/projects/{id}`, `/projects/{id}/claims` grant/transition/release/events,
`/projects/{id}/work-items` list/create/close, `/projects/{id}/next` with
DB-first grant and best-effort label projection), and tests (store unit
tests including the concurrent-grant race, API tests including a scripted
fake tracker). A second slice landed the same day: the `fabro_work_next/show/create/
close/release` run tools (engine-mediated, catalog + dispatch +
fabro-client wrappers), the `work.claimed`/`work.released` platform
records (CLI + web rendering), and the auth seam (project config
user-only; claims/items/next accept the `agent:run_tools` worker). A
third slice closed Phase 1: lane backpressure (`awaiting_merge` claims
block the lane's next grant, cap `backpressureOpenPrCap` per project,
fabro-4580), the work reconciler (outbox drain with backoff, DB->tracker
projection healing, run-status-driven awaiting_merge/release with a
30-minute orphan grace instead of the 6-hour timer, run-id label GC),
`current_work_item` in the run projection (folded from
`work.claimed`/`work.released`), and the ADR-0031 merger closing the
landed work item + landing its claim, and releasing a duplicate's claim.
Phase 1 code is complete; what remains before Phase 2 is the live
dry-run against a real tracker repository.

1. `ProjectRegistry`: project id, display name, tracker provider, tracker
   repo, project/lane/type/priority label maps, claim label prefix, code
   repos, default branch, merge policy. Server-side source of truth; a run
   declares only `project = "<id>"`.
2. `ClaimLedger`: schema and migrations; partial unique indexes; grant,
   release, transition, events; `awaiting_merge` semantics.
3. `Outbox`: idempotent projection jobs; retry/backoff; dead-letter
   visibility.
4. `Reconciler`: startup and periodic; DB -> provider and provider -> DB
   (rebuild mode); label GC.
5. `TrackerProvider` trait and `NullTracker`; a `FakeTracker` test double.
6. API (project-scoped): work-item list/show, claim grant/release, queue
   view. Regenerate the Rust client (`cargo build -p fabro-api`) and the
   TypeScript client.
7. Run projection: `current_work_item` (project + item key) alongside the
   existing `current_seed_id`; `work.claimed` / `work.released` platform
   records and their stream rendering.
8. Run tools: `fabro_work_next`, `fabro_work_show`, `fabro_work_create`,
   `fabro_work_close`, `fabro_work_release`; node allowlist pins in the
   workflows; ADR-0019 approval recorded in the ADR.
9. Backpressure: grant refusal while the lane has open run PRs; cap 0 as
   the default (fabro-4580).
10. Tests: uniqueness under concurrency; release on terminal/merge/
    duplicate/external close; awaiting-merge blocks re-claim; project
    scoping; provider contract; outbox retry; reconciler both directions;
    rebuild from labels; backup/restore drill.

No workflow prompt changes in Phase 1; the new path is dark until the
adapter and the dry-run tracker exist.

## Phase 2 — GitHub adapter and tracker repository

1. Create the private tracker repository `denkhaus/work`; add a README,
   issue templates for user/revisor/architect filings, and the label set.
   Do not enable per-repo issues in code repositories.
2. `GitHubTracker`: list (labels, pagination), get, create, close,
   comment, project claim, clear claim. GitHub App installation on the
   tracker repo with `issues: write` and `metadata: read`; add Projects
   permissions only if the board is in scope.
3. Label mechanics: create the label object first, then attach it
   (GitHub returns 422 for unknown labels); treat "already exists" as
   success; delete on release; GC unused `fabro:run:*` labels.
4. PR link: the run PR body gets `Fixes <tracker>#<n>`; the server writes
   the PR URL to the issue; the merger closes the issue by API because the
   base branch is not the repository default.
5. Contract tests against a sandbox repository or a scripted HTTP double:
   label create/attach/remove/delete, 404/422/429 handling, pagination,
   idempotency, cross-repo links.
6. A dry-run project (`project:fabro-dryrun`) in the tracker repo with a
   handful of synthetic items, used by the workflow tests.

## Phase 3 — Workflow rewrite

Replace every tracker read/write with the run tools and the project
config. Key files:

- `develop`: `prompts/planner.md`, `prompts/tracker-commands-planner.md`,
  `prompts/implementer.md`, `prompts/reviewer.md`,
  `scripts/planner-preflight.nu`, `scripts/claim-check.nu`,
  `scripts/tracker-guard.nu`, `scripts/closeout.nu`,
  `scripts/closeout-race.nu`, `scripts/evidence.nu`,
  `scripts/gate-bounce.nu`.
- `loop`: `prompts/planner.md`, `prompts/implementer.md`,
  `prompts/reviewer.md`, `prompts/project-facts.md`.
- `revisor`: `prompts/file.md` (file via `fabro_work_create`),
  `prompts/analyze.md`, `prompts/select.md`, `prompts/facts.md`,
  `scripts/closeout-race.nu`, `scripts/overflow-ledger.nu` (retire or
  reduce to a report).
- `architect`: `prompts/file.md`, `prompts/survey.md`,
  `prompts/analyze.md`, `prompts/facts.md`.
- `conductor`: `prompts/*` (survey/merge/revise legs) that count or name
  seeds.
- shared: `.fabro/scripts/dup-run-check.nu` (read server state, not
  `seeds show`), `salvage-sweep.nu`, `close-claim-check.nu`,
  `friction-score.nu`, `prompt-lint.nu`; seed fixtures under
  `.fabro/scripts/*-fixtures.nu`.
- docs and skills: `AGENTS.md`, `CLAUDE.md`, `.agents/skills/iterate`,
  `.agents/skills/integrate`, workflow `project-facts.md` files.

Rules for the rewrite (user direction 2026-10-20):

- No prompt names a tracker vendor, repository, label or ID scheme; it
  names the run tools and the project.
- **No `gh` anywhere in the line's assets.** `fabro_work_*` + the
  project-scoped API remove the last reason for it. The toolchain image
  STAYS gh-free (ADR-0019; the Dockerfile.toolchain pin from 2026-09-08
  must survive this migration — nothing gets re-added). Host-side nu
  scripts that still shell out to `gh` are slimmed onto the server API:
  `push-gate.nu` (open-PR check -> the lane's queue/PR view),
  `closeout.nu`, plus the tracker-parsing scripts
  (`dup-run-check.nu`, `planner-preflight.nu` arms, `claim-check.nu`,
  `tracker-guard.nu`, `close-claim-check.nu`, seed fixtures) — their
  seeds/JSONL parsing collapses into `fabro_work_*` calls or retires.
- Prompts that mention `gh` today (`develop/planner.md`,
  `develop/reviewer.md`, `revisor/file.md`, `pr-simplify/simplify.md`,
  `patch-cves/patch-cves.md`) are rewritten onto the tools in the same
  unit as their workflow's script changes (ADR-0008 one-unit rule).
- The planner's candidate source is `fabro_work_next` (tool grant), not
  `seeds ready`. A misrouted/stale item is released with
  `fabro_work_release` and the planner asks again.
- Closeout closes through `fabro_work_close`; the merge close is the
  merger's job. Do not double-close.
- Revisor/architect filing uses `fabro_work_create`; the ADR-0022 balance
  is counted by the server or by the existing script against the new
  create/close events.
- `sensitive` items: the planner writes a neutral run title and the
  implementer/reviewer never copy issue details into commit subjects or PR
  titles. The run/PR title is public when the code repo is public.
- Verification commands stay parse-level in the implementer and full
  battery in the tester, as today.

### Phase 3 settled forks (grill-with-docs 2026-10-21, user GO on all)

Jev pre-screens ran per question (advisory; framing-bias named; recorded
in the 2026-10-21 judgment log). Chosen path and rejected alternatives:

1. **Dedupe search** — chosen: server-side `query` parameter on
   `list_work_items` (endpoint + `fabro_work_list`): tracker text search
   over title+body, INCLUDING closed items. Rejected: open-only dedupe
   (re-file risk for closed work — the ADR-0027 inventory showed ~74%
   rebuild-of-existing classes) and host-side presolve (cannot see
   mid-run filing races). Engine unit lands BEFORE the workflow units.
2. **Legacy-id resolution** — chosen: `fabro_work_show` accepts a legacy
   id; the server resolves it through the forge API via the
   `seeds:fabro-xxxx` label (exact; body text search stays the
   human-search path). Seam lands with the Phase-4 importer unit (labels
   exist only after import). Rejected: host dispatch-time translation
   (silently rewrites user text) and fail-closed refusal (strands old
   references).
3. **overflow-ledger.nu** — chosen: KEEP as-is (the ADR-0022 noise
   filter survives; only `filed-as` markers change format to item keys,
   fixtures/pins follow). Rejected: retire-with-file-everything (tracker
   flood; backlog stagnation is already the top friction component) and
   reduce-to-report.
4. **push-gate open-PR check** — chosen: the project claims view
   (`granted`/`active` = running pass, `awaiting_merge` + `prUrl` = open
   run PR; refusals name claims and PR urls). Foreign/human PRs no longer
   block (the GitButler lane-PR class is empty; residual risk documented
   in the gate header). The `fabro ps` active-run arm STAYS (catches
   non-tracker runs). Rejected: new repo-PR endpoint (API surface beyond
   ADR-0032's pipeline view) and runs-scan.
5. **push-gate `.seeds` integrity check** — chosen: KEEP while the frozen
   file remains in the repo; drop it in the SAME commit that moves the
   file to the tracker archive (the path check would refuse forever
   otherwise). Rejected: drop-at-freeze (rebase can still corrupt the
   frozen history) and replace-by-tracker-health-probe (couples the gate
   to GitHub availability).
6. **Dead gh demo workflows** — chosen: DELETE `gh-list`, `gh-triage`,
   `implement-issue` (gh-verification and pre-tracker demos; the
   toolchain has been gh-free since ADR-0019). `merge-upstream` stays
   untouched (ADR-0027 freeze; it never runs).
7. **Tracker data into in-sandbox deterministic scripts** — chosen:
   context flow. LLM stages fetch via `fabro_work_*` and forward through
   `context_updates` -> `stdin_source`; the engine injects the grant
   outcome deterministically as a `current_work_item` context key
   (replaces `current_seed_id` injection; small engine seam). The
   pre-planner preflight loses its `seeds ready` candidate list and
   checks the GRANTED item instead; candidate iteration becomes the
   planner's existing release-and-ask-again loop. Rejected: run-scoped
   bridge CLI (new engine surface + ADR-0019 review + duplicate agent
   surface) and retiring tracker arms wholesale (loses the exact-claim
   binding of the evidence gate).
8. **closeout-race.nu** — chosen: RETIRE both copies (develop, revisor);
   the rule becomes a mandated prompt step: the filing stage re-runs
   `fabro_work_list` with `query=<target run id>` fresh at filing time
   (the freshness IS the race guard). Rejected: thin-wrapper rewrite on
   stdin input (script shell without added determinism).

## Phase 4 — Data migration

1. Freeze `.seeds/issues.jsonl` at a known commit; record the commit sha.
2. Write an idempotent importer:
   - open items first, then closed;
   - title, body, labels, priority, type, assignee-ish metadata, blockers
     (as a structured `## Blocked by` section or native dependencies if
     available);
   - `Legacy-ID: fabro-xxxx`, `Original-Created`, `Original-Closed`,
     `Migrated-From: denkhaus/fabro@<sha>`;
   - closed items created and closed with `state_reason=completed`;
   - the legacy id label `seeds:fabro-xxxx` on every migrated issue
     (amendment 2026-10-21: the label replaces the originally planned
     mapping file — exact label lookup resolves old references; the
     resolver also accepts the bare `fabro-xxxx` form and namespaces it);
   - a count/parity report: N source records at freeze (938 at the
     2026-10-20 decision; recount after this epic and fabro-b8b7 are
     filed), N created issues, N `seeds:*` labels (must equal N), 0
     duplicate legacy ids, 0 parse errors.
3. Archive the JSONL in the tracker repo
   (`archive/issues-<date>.jsonl`), so the tracker repository is the one
   place for items and history.
4. Dry-run the importer against `project:fabro-dryrun` and diff the
   normalized items against the source.

## Phase 5 — Cutover

- [ ] Configure the line's project: `tracker = github:<owner>/<repo>`,
      `project = fabro`, label maps, claim prefix.
- [ ] Switch the develop/loop/revisor/architect/conductor workflows to the
      new tools (one PR, reviewed; the line is off).
- [ ] Remove `seeds` from the line's sandbox toolchain and prompts; keep
      the `seeds` product and adapter for other projects.
- [ ] No `gh` call sites remain in `.fabro/` (scripts or prompts); the
      toolchain image stays gh-free (ADR-0019 pin in
      `.fabro/Dockerfile.toolchain` unchanged).
- [ ] Freeze `.seeds` in the Fabro repo (no new writes; leave the file or
      move it to the tracker archive in a separate commit).
- [ ] Update `docs/internal/line-restore-handoff.md` and the iterate/
      integrate skills to the new tracker.
- [ ] Run the verification matrix below.

## Phase 6 — Verification and re-arm

**Deterministic tests**

- [ ] Two concurrent grants for one item: exactly one wins.
- [ ] One run cannot hold two active claims.
- [ ] `awaiting_merge` blocks a second claim until the PR merges or closes.
- [ ] Release on run failed, run cancelled, duplicate close, external
      close, orphan (30-min grace).
- [ ] Backpressure: a lane with an open run PR refuses a grant.
- [ ] Reconciler: DB -> tracker, tracker -> DB, label GC.
- [ ] Crash drill: kill after DB commit before projection; restart;
      projection healed; no duplicate grant.
- [ ] DB restore from snapshot; rebuild from labels after simulated DB
      loss.
- [ ] GitHub outage drill: outbox retries; active runs continue.
- [ ] Provider contract: FakeTracker, GitHubTracker sandbox repo,
      SeedsTracker read-only.
- [ ] Migration parity: N/N at freeze, 0 duplicate ids, 0 parse errors.
- [ ] Sensitive title check: a `sensitive` item's run/PR/commit title is
      neutral.

**Live drills (manual, one at a time; never two in flight)**

- [ ] One manual develop run on a real item: grant, label, PR `Fixes`,
      merge close, claim `landed`.
- [ ] One manual revisor run: files an item via `fabro_work_create`;
      no stranding; balance behavior verified.
- [ ] One manual failed run: claim released; item claimable again.
- [ ] One duplicate scenario: a second run cannot claim the same item;
      the duplicate-close path releases correctly.
- [ ] Check the issue timeline: label add/remove, PR link, close.

**Re-arm checklist (all boxes, then ask the user for GO)**

- [ ] ADR-0032 and this plan accepted; Phase 1-5 complete.
- [ ] Verification matrix green at HEAD.
- [ ] Backpressure active (cap 0).
- [ ] `current_work_item` in the run projection.
- [ ] The old seeds-specific preflight arms are retired or explicitly
      inert; no workflow still reads `.seeds`.
- [ ] User GO.
- [ ] Re-arm revisor-fabro and loop-fabro only; develop fires through the
      revisor cascade; architect/conductor stay off unless the user says
      otherwise.
- [ ] 24h acceptance: zero uncontained duplicates; every filed finding
      reaches the tracker; every landed PR closes its issue; no stale
      claim survives a terminal run.

## Rollback

- The tracker repository is additive; migration does not delete source
  data. Keep the frozen JSONL and the importer mapping until the first 10
  clean cycles.
- The workflow switch is one reviewed commit; revert it if the new path
  fails before any schedule is re-armed.
- If GitHub is unavailable during cutover, keep the line off; do not fall
  back to file claims.
- If the claim ledger is the failing component, disable the scheduler and
  run manual single fires only; never two in flight.

## Risks and open questions

- **GitHub App permissions on a personal repo:** issue writes are fine;
  Projects v2 may need a different permission/owner. Verify before
  promising a board.
- **Cross-repo integration:** the PR "Development" sidebar is weaker
  across repositories; the server writes the links. Accepted.
- **Central tracker as a single point of failure:** the outbox and the
  cache bound it; a full GitHub outage stops new grants, not active runs.
- **Historical timestamps:** GitHub cannot set `created_at`/`closed_at`;
  the body carries the originals and the JSONL archive is authoritative
  for history.
- **Public git history of `.seeds`:** removal does not scrub history; a
  separate decision if needed.
- **ADR-0023:** the seeds product remains, but the line stops dogfooding
  it. Revisit the native-seeds roadmap explicitly rather than letting the
  two directions drift.
- **Project registry location:** server-side source of truth; a
  `.fabro/project.toml` may carry only the project id. Do not duplicate
  tracker config in every code repo.

## File inventory (starting point, verify at HEAD)

- Server: new modules under `lib/apps/fabro-server/src/` (project registry,
  claim ledger, outbox, reconciler, tracker providers); OpenAPI additions
  in `docs/public/api-reference/fabro-api.yaml`; run tools in
  `lib/components/fabro-workflow/src/run_tools.rs` and the server's tool
  dispatch; projection in `lib/apps/fabro-server/src/server/`.
- Workflows: `.fabro/workflows/{develop,loop,revisor,architect,conductor}/`
  as listed in Phase 3.
- Scripts: `.fabro/scripts/{dup-run-check,salvage-sweep,close-claim-check,
  friction-score,prompt-lint}.nu` and the seed-specific fixtures.
- Docs: `docs/lab/adr/0032-central-work-item-tracker.md`,
  `docs/internal/central-work-tracker-migration.md`,
  `docs/internal/line-restore-handoff.md`, `AGENTS.md`, `CLAUDE.md`.
- Skills: `.agents/skills/iterate/SKILL.md`,
  `.agents/skills/integrate/SKILL.md`.
- Toolchain: `.fabro/Dockerfile.toolchain`, `.mise.toml`, `justfile`
  (remove the line's seeds dependency only after cutover).
