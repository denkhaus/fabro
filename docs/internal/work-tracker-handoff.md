# Handoff: central work-item tracker — Phase 1 complete, Phase 2 next

Written 2026-10-20 (late) on **v1000** by the grill/implementation session.
Audience: the next host-side agent session. Chat language with the user:
German. All user-facing times: Europe/Berlin.

Authority: `docs/lab/adr/0032-central-work-item-tracker.md` and
`docs/internal/central-work-tracker-migration.md` (the plan). This document
is the session handoff: what landed, what is verified, what is external,
and the exact next actions.

## Situation

- The autonomous line stays **OFF** (loop/revisor/architect/conductor
  schedules off). Do not re-arm before the plan's Phase 6 checklist + user
  GO.
- **Phase 1 is code-complete and committed locally, NOT pushed.** Branch
  `denkhaus`, `git log` top: `4a91769e3` (backpressure, reconciler,
  current_work_item, merger landing), `d60eedb92` (fabro_work_* tools +
  work.claimed/released records + auth), `9d77460dd` (fabro-work crate +
  claim ledger + GitHub adapter + API), `446eeccab` (ADR + plan),
  `baf57df00` (plan status), `3c20089df` (fabro-ae49).
- **External state created:** private repo `denkhaus/work` (README pushed,
  label set created: project:*, lane:*, prio:p0..p4, type:*, source:*,
  needs-user/revision/sensitive/blocked), and dry-run issue **#1**
  (`project:fabro-dryrun`). The GitHub App is **NOT yet installed** on
  `denkhaus/work` — the server cannot mint a token for it until it is.
- The production server (mirtuell) runs the previous release: the new
  endpoints/tools exist only in this tree. No workflow has been switched.

## What landed (Phase 1 map)

- **`lib/components/fabro-work`** (new, forge-agnostic): `WorkStore`
  (`work_projects`, `work_claims` with partial unique indexes — one live
  claim per item and per run, `work_claim_events`, `work_outbox`),
  `TrackerProvider` port + `WorkItem` types + `NullTracker`.
- **`lib/components/fabro-github/src/work_tracker.rs`**: `GitHubTracker`
  (issues, run-id label projection, label GC). Forge #2 (Forgejo) is one
  adapter + one match arm in `AppState::work_tracker_for_project`.
- **SQL migration** `lib/foundation/fabro-db/migrations/2026102002_work_queue.sql`
  (edited in place; unreleased, so safe).
- **API** (OpenAPI-first, `docs/public/api-reference/fabro-api.yaml`):
  `/api/v1/projects` CRUD, `/projects/{id}/claims` (grant/transition/
  release/events), `/projects/{id}/work-items` (list/create/close),
  `/projects/{id}/work-item` (single), `/projects/{id}/next` (atomic
  grant; outcomes granted/already_held/empty/**blocked**).
- **Auth**: project config = `RequiredUser`; claims/items/next =
  `RequiredRunManagementActor` (User or `agent:run_tools` worker; plain
  worker 403).
- **Tools** (`fabro-tool` + dispatch in `fabro-workflow`):
  `fabro_work_next/show/create/close/release`, engine-mediated,
  `fabro_work_next` defaults the run to the session's own.
- **Records**: `work.claimed` / `work.released` (typed payloads, projection
  fold, CLI stream rendering, web panel labels). `current_work_item` in
  `RunProjection` (beside `claimed_seed_id`), folded from the records.
- **Reconciler** `lib/apps/fabro-server/src/server/work_reconciler.rs`
  (spawned in `serve.rs`, 30s tick, aborted on shutdown): outbox drain with
  linear backoff, DB->tracker projection healing, terminal-run →
  awaiting_merge / release, missing-run release after **30 min** grace, label
  GC. No 6-hour tracker guard.
- **Backpressure**: `backpressureOpenPrCap` per project (default 0 = fire
  only when drained); a lane's next grant refuses while it has
  awaiting_merge claims, returning `blocked` + the blocking claim
  (fabro-4580).
- **Merger** (ADR-0031, `fork_staleness_supervisor.rs`): a merged run PR
  closes its work item (reason cites the PR), clears the label, lands the
  claim, appends `work.released`; the exact-duplicate close releases the
  duplicate's claim. `Projector::startup_pass` body is boxed (the growing
  fold tipped `large_futures`).
- **Docs**: ADR-0032, migration plan (status blocks updated), line-restore
  handoff marked superseded in part, `fabro-a33e` epic (store migration) +
  `fabro-b8b7` epic (claim authority, decisions recorded), `fabro-ae49`
  (pre-existing petri failures).
- **TS client** regenerated in `lib/packages/fabro-api-client`.

## Verified state (at `4a91769e3`)

- `cargo check --workspace` clean.
- clippy `-D warnings` clean across fabro-work/github/store/types/petri/
  server/tool/client/workflow/cli.
- rustfmt clean (pinned nightly).
- nextest: **fabro-server it 1140/1140** (1 skipped); fabro-work +
  fabro-store + fabro-github + fabro-tool + fabro-client + fabro-workflow
  + fabro-types **795/795**; fabro-work store tests include the
  concurrent-grant race (16 grants → one winner); reconciler tests (outbox
  heal, 30-min orphan grace); backpressure it-test (blocked while
  awaiting_merge, grants after landing).
- **Pre-existing failures, NOT ours:** 19 fabro-petri tests fail
  identically at baseline HEAD (verified with a clean worktree) — filed as
  **fabro-ae49**. They block a clean gate later. Also removed: the stale
  `tests/it/api/seeds.rs` (mirror-era `/api/v1/seeds`; presence pin
  `fork_run_scoped_seeds_api_tests` guards the removal).

## Next actions, in order

0. **Push gate.** Confirm 0 open `fabro/run/*` PRs (true at handoff time)
   and push `origin denkhaus` — the commits exist only on v1000. Then
   `git pull --ff-only` on any other host. Do not push while run PRs are
   open.
1. **Deploy** the new build to mirtuell (or run it locally) so the new
   endpoints/tools exist. Install the GitHub App on `denkhaus/work`
   (user action; needs `issues: write`, `metadata: read`; Projects only if
   a board is wanted).
2. **Phase 2 live verification** (no line): `PUT /api/v1/projects/fabro`
   with `trackerProvider=github`, `trackerRepo=denkhaus/work`,
   `projectLabel=project:fabro`, `laneLabels={loop:lane:loop,...}`,
   `claimLabelPrefix=fabro:run:`, `backpressureOpenPrCap=0`. Then exercise
   issue **#1** (`project:fabro-dryrun`): grant via
   `POST /projects/fabro/next` with a real run id, verify the
   `fabro:run:<id>` label appears, release, verify the label is removed and
   GC'd, create/close a throwaway item, and check the reconciler no-ops on
   a healthy ledger. Record the results in the plan's Phase 6 matrix.
3. **Phase 3 workflow rewrite** (the big one; one-unit rule): develop/
   loop/revisor/architect/conductor prompts + scripts onto `fabro_work_*`;
   no `seeds` writes, no `gh` anywhere (host `push-gate.nu` open-PR check
   moves onto the queue view; `dup-run-check.nu`, preflight arms,
   `claim-check.nu`, `tracker-guard.nu`, closeout/close-claim scripts slim
   or retire; prompts mentioning gh: develop/planner.md,
   develop/reviewer.md, revisor/file.md, pr-simplify/simplify.md,
   patch-cves/patch-cves.md). Then `prompt-lint`, `validate`, targeted
   tests. The line stays off; land via manual single fires or host-side.
4. **Phase 4 data migration**: idempotent importer for all items (open
   first; closed with `Original-Created`/`Original-Closed`), `Legacy-ID:
   fabro-xxxx`, mapping file, archive JSONL into `denkhaus/work/archive/`,
   parity N/N at freeze (938 at decision time), 0 duplicate ids.
5. **Phase 5 cutover**: freeze `.seeds`, remove seeds from the line's
   toolchain/prompts, gh-free check, update skills (`iterate`,
   `integrate`) and `AGENTS.md`/`CLAUDE.md`.
6. **Phase 6**: verification matrix + crash/recovery drill + one manual
   develop and one manual revisor run; then the re-arm checklist and user
   GO. Re-arm revisor + loop only.

## Do not

- Do not re-arm schedules; the line stays off until Phase 6 + GO.
- Do not re-add `gh` to `.fabro/Dockerfile.toolchain` (ADR-0019 pin) and do
  not give agents a forge token: writes stay engine-mediated.
- Do not use a boolean claim label or a claim comment: the projection is
  the run-id label `fabro:run:<id>`; the DB is the authority.
- Do not hard-code `denkhaus/work`, `fabro`, or GitHub into the core: the
  port and store are forge-agnostic by user directive (Forgejo later).
- Do not rewrite the already-public `.seeds` git history as part of this
  migration (separate decision).

## Operational facts

- Host: **v1000** (more resources than weblab; no issue running the full
  suites here). Production server: `FABRO_SERVER=https://mirtuell.net`.
- `gh` auth works as `denkhaus`. The fabro checkout's SSH origin uses an
  alias (`git@denkhaus.github.com:...`); pushing new repos from
  `/tmp/work-tracker` needed the token URL
  (`git push https://x-access-token:$(gh auth token)@github.com/denkhaus/work.git main`).
- Tracker repo: `denkhaus/work` (private). Issue #1 is the dry-run probe.
- Commands used for verification:
  - `cargo +nightly-2026-04-14 clippy -p fabro-work -p fabro-github -p fabro-store -p fabro-types -p fabro-petri -p fabro-server -p fabro-tool -p fabro-client -p fabro-workflow -p fabro-cli --all-targets --features test-support -- -D warnings`
  - `cargo nextest run -p fabro-server --features test-support --no-fail-fast`
  - `cargo nextest run -p fabro-work -p fabro-store -p fabro-github -p fabro-tool -p fabro-client -p fabro-workflow -p fabro-types --no-fail-fast`
- Tracker (`seeds`) is still the source for the remaining work until
  Phase 4/5: `seeds show fabro-a33e fabro-b8b7 fabro-ae49`.
