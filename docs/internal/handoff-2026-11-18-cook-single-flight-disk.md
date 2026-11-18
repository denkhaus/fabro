# Handoff: Toolchain cook single-flight & the Docker disk (2026-11-18)

Updated: 2026-11-18, 15:45 CET (14:45 UTC) — PR #602 merged to trunk
(`1d979693d0`); the fix goes live only with the next server deploy.

Origin: a line-monitoring session on 2026-11-18 (runs of the morning, painpoint
sweep, live incident follow-up). This document freezes the state, the decisions,
and the open actions for whoever picks the thread up next — operator, loop
session, or a future agent. All times UTC unless marked CET.

Work items referenced below are keys in project `fabro` on the production
tracker (`denkhaus/work`), driven by `fabro work` (see
`docs/agents/issue-tracker.md`).

## TL;DR

Yesterday evening (2026-11-17) the deploy host's Docker disk hit **57 MB free**
and needed an emergency purge — the 4th disk-full of the class. The root defect
on the mint side: concurrent gear laps that miss a not-yet-built `petri-env-*`
image tag each run a FULL build of the same image (no concurrent dedup), and
nothing in normal operation ever evicts old TAGGED env images (173 accumulated).
The decided fix carries as **work#1440 (p0, lane:develop)**: an in-process
single-flight build guard + a mint-time LRU ceiling, one vendored petri change.
The develop lap landed it the same morning: PR #602 merged at 14:40 UTC
(trunk top `1d979693d0`, `just vendor-pins` verified green on the merged
tree). It goes live only with the next server deploy — the cook executor
compiles into the fabro-server binary. Everything else in the cluster has
exactly one carrier each (Line map below) — the user directive was one line
per surface, not parallel items.

## State at handoff (updated 2026-11-18, 14:45 UTC)

- **work#1440** (p0, develop, type:bug): implementation MERGED — PR #602
  (`1d979693d0`, diff +572/−46 in 13 files), lap 30m55s, reviewer verdict
  Approved. `just vendor-pins` verified green on the merged tree. The ITEM
  stays open by design: its closure radius holds work#1188, which carries
  Demand 3's backstop half and Demand 4 (the alarm shipping before the next
  57 MB incident) — unlike the work#1418 case, that interlock is semantically
  correct. The CODE goes live only with the next server deploy (the cook
  executor compiles into the fabro-server binary; the running fork.40 build
  from 2026-11-17 predates it).
- The lap's open residuals, carried as observations on the run
  (`01M7QEQT1P8CF3SF4D4W8SN7BE`): the ADR-0027 fork export is DONE (see the
  Todo — the run's "export pending" painpoint was stale); pre-existing
  nightly-clippy debt in vendored crates (`large_enum_variant` in petri-ir)
  is not this change's debt.
- **work#1418** (soft-exit prose, p1): CLOSED — the fix merged 11:40 UTC as
  PR #600; the item had been stuck open on a `close_radius_open` retry loop and
  was freed by unlinking its closure-radius links (see "The radius lesson").
- **work#1421** (petri test schema, p1): CLOSED — its residual fix merged
  12:42 UTC as PR #601; freed the same way.
- **work#1371** (single-flight feature proposal): CLOSED as superseded by
  #1440 (re-prioritized p0 on the disk-fill dependency).
- **work#1353** (cook lap duration, loop): CLOSED as duplicate of work#1335;
  its misroute disclosure survives verbatim in #1335's "Surface note".
- **work#1441** (2h churn sweep, triager-auto-filed at 14:01 UTC): CLOSED as
  duplicate of work#1188 the same hour — the sweep rides the SAME watchdog
  tick and belongs to the same disk-hygiene surface; its guard design (sweep
  only under df pressure / above a reclaim floor, respect the work#945 Part A
  retention window, never touch non-terminal runs, per-tick reclaim logging)
  survives verbatim as #1188's Facet 2.
- Deploy host (netcup-5) post-purge: vdb (/var/lib/docker/overlay2, 503 GB)
  at 53%; 173 petri-env tags / 59 unique images / 220.4 GB; containers
  41.21 GB (99% reclaimable); volumes 183.1 GB. These tags stay until the
  deployed binary's first eviction sweep (the host GC does not touch tagged
  images).

## Layer 1 — the incident (2026-11-17, work#1425)

The 4th disk-full of the class (predecessors: work#737, work#945, work#1185).
Timeline highlights: 20:44 host recon found the churn NVMe at 85–89% with
~19 GB/day growth; ~22:1x the overlay hit FULL faster than projected (a
developer run was mid-build on the host, its cold cargo build writing GBs of
layers on top); the operator stopped all 5 gears and the building run. #1425
carries the label-safe relief recipe (exited petri containers → volume prune →
dangling + label-scoped image prune) and the re-arm checklist. Its relief step 5
— activating the landed work#168 retention arm (seed fabro-tofu-1055,
`terminal_sandbox_age_hours 48`, `disk_brake_percent 85`) — was still owed at
handoff time.

## Layer 2 — the mint side (the defect #1440 fixes)

`vendor/petri` `executor-sandbox/src/image_build.rs`:

- Dedup is SEQUENTIAL only: content hash over Dockerfile + packed context →
  tag `petri-env-<hash12>`; a `Build { tag, reuse: true }` short-circuits on
  tag presence.
- No CONCURRENT dedup: N runs that all miss the tag before the first builder
  finishes each pack, spin a holder, and run the whole build ("Docker merges
  nothing across build sessions"). The module docs acknowledge the window
  ("... before the `reuse` check can see the tag").
- `pack_context` tars the entire checkout on every cook, including reuse hits.

Live specimens from 2026-11-18: after PR #600 merged (11:40 UTC) the 11:45 UTC
triager+adjudicator laps sat **22m04s/22m15s** in Start; after PR #601
(12:42 UTC) FOUR gears cooked the same tip concurrently and the adjudicator lap
exceeded its 31m stall ceiling; the 13:00 UTC watchdog then alarmed "last
completed pass 50min ago, window 45min" for three gears. Duplicate-build
thrash scales with N.

Decided target behavior (user, 2026-11-18, verbatim): "Wenn ein Gear in den
Prozess des Image-Neubaus gerät, wird der Neubau für alle anderen Gears
geblockt; wenn das erste Gear mit dem Neubau fertig ist, nutzen die anderen
Gears das von diesem gebaute Image automatisch." — single-flight blocking +
automatic reuse, via a process-wide `tag -> shared build guard` in
`ensure_environment_image` (in-process lock suffices; all gears cook in the
same fabro-server process). Plus the rider: hash a context manifest (file list
+ sizes + mtimes) instead of the packed tar.

## Layer 3 — the disk math (why p0, why the cadence decision)

220.4 GB across 59 unique images ≈ **3.7 GB marginal per trunk tip**; the line
merges ~hourly (`docs/agents/checkout-pipeline.md`) → **~24 tips/day ≈ 70–90
GB/day unpruned growth** → vdb fills in 2–3 days of no purging. User decision:
a 24h purge cycle is definitively too late; ~2h is conceivable — but the
cadence is the BACKSTOP only. The primary lever is event-driven eviction at
mint time (Demand 3 of #1440): after a successful cook, evict least-recently-
used petri-env images beyond a ceiling (keep last ~6 or ~60 GB of env-image
bytes). The cadence never carries the mint load; the ceiling bounds the disk
by construction.

## Layer 4 — what already exists (do not build a second sweeper)

- **Host GC cron** `/etc/cron.d/fabro-toolchain-image-gc` (hourly at :17,
  tofu-provisioned, script `/opt/fabro-ops/toolchain-image-gc.nu`): prunes
  dangling images, 3d-stale build cache, anonymous volumes, corpse containers
  (the 2026-11-10 cargo-panic class); df readings every run with WARN and an
  emergency `--all` arm at the disk threshold. STRUCTURAL GAP: its regular
  arms never evict TAGGED petri-env images — the 173-tag accumulation happens
  entirely inside its blind spot (log 2026-11-18 14:17 UTC: reclaimed 0 B
  while 173 tags sat there). #1440's mint-time ceiling covers this class
  engine-side; deliberately NO GC-extension item — one implementation.
- **Retention supervisor** (work#168, landed, NOT yet activated): terminal
  sandbox reap + disk brake at 85%, oldest-terminal-first. Covers containers
  and sandboxes, not tagged env images. Activation is owed (Layer 1).
- **A second cron** `/etc/cron.d/fabro-scratch-gc` (daily 03:42): reaps
  `scratch/<YYYYMMDD>-<runid>` dirs older than 30d — the work#168 bridge
  ("Switch off with work#168").
- **Watchdog** (hourly gear): pass-completion stall check alarmed correctly at
  the extreme (the 50-min alarm) but is blind to Start-stage stalls in
  general — it certified "all healthy" at 12:17 UTC while itself sitting 17m41s
  in Start. The lap-latency facet is work#1438.

## Layer 5 — the radius lesson (resolved same-day, recorded as policy debt)

The 2026-11-18 morning showed a different Henne-Ei: work#1418's fix was LIVE
on trunk (PR #600) while the item stayed open — its closure radius held #1421
and #1431, and the adjudicator's closing pass retried `close_radius_open`
every 15 minutes without backoff. The operator emptied both radii via
`fabro work unlink` (four links; all were independent follow-ups or
already-landed residuals), and the next closing pass closed both items. The
policy fix (backoff + radius semantics for follow-up links) is **work#1434**.
One-line-per-surface note: #1440's Line map section in the tracker is the
authoritative carrier of the cluster's surface split.

## Line map (one carrier per surface — user directive 2026-11-18)

| Surface | Carrier | Lane | State at handoff |
|---|---|---|---|
| Env-image MINT: single-flight guard + mint-time LRU ceiling | **work#1440** (p0) | develop | merged to trunk (PR #602); deploy pending |
| 2026-11-17 incident record + relief recipe + #168-arm activation | work#1425 (p1) | loop | open |
| Retention supervisor for terminal run containers/sandboxes | work#168 (p2) | — | landed, activation pending |
| df threshold alarm + hourly backstop executor | work#1188 (p2) | loop | open |
| Per-tip cook COST (release-binary fetch; loop assets: `.fabro/Dockerfile.toolchain`, `just host-release`) | work#1335 (p1) | loop | open |
| Build-engine routing (BuildKit; classic builder cannot serve tick cadence) | work#1272 (p2) | — | open |
| Toolchain image layer order (mise below cargo cook) | work#1208 (p2) | — | open |
| Watchdog alarm channel | work#1168 (p1) | loop | open |

Closed within this cluster: work#1371 (superseded by #1440), work#1353
(duplicate of #1335), work#1441 (duplicate of #1188, guard design folded
into #1188 Facet 2), work#945 / #737 / #1185 (prior incidents of the class).

## Same-session adjacent filings (distinct lines, no overlap with the map)

Filed 2026-11-18 from the same monitoring session, each its own surface:
work#1434 (close_radius_open backoff + radius semantics, p1 develop),
work#1435 (gear-fire holdover: an overrunning lap eats the next scheduled
fire; kinship work#1411), work#1436 (gear admission gate: skip run creation
when the server-side worklist is empty), work#1437 (fabro ps graceful
degradation on PR-field skew + self-update from the server image),
work#1438 (watchdog lap-latency incl. Start-stage), work#1439 (agent output
contract accepts outcome synonyms).

## Complexity notes (why this is not one small fix)

1. The guard/ceiling is a VENDORED petri change (executor-sandbox) — it
   exports to the fork per ADR-0034/ADR-0027 in the same change that needs it,
   and the toolchain image cooks `vendor/` from the repository itself
   (ADR-0042), so the edit reaches the cook without extra staging.
2. Develop-landable despite being petri: vendor/petri paths are proven develop
   surface (work#1421's test-schema fix landed via PR #601).
3. The per-tip cook COST fix (#1335) sits on LOOP assets — develop's fs
   envelope cannot land `.fabro/Dockerfile.toolchain` or the justfile
   (disclosed as a misroute on 2026-11-14, preserved in #1335's Surface note).
   Cheap regen is also what makes aggressive eviction safe — the two items
   are complementary, not redundant.
4. The host GC is tofu-provisioned infrastructure with an explicit safety
   design (docker never prunes container-referenced images; the floor keeps
   rollback images; corpse containers leave only after a debug window and only
   when provably orphaned). Raising its aggressiveness is NOT the chosen path —
   the engine-side ceiling is.

## Todo

Implementation (landed on trunk; fork export and deploy owed):

- [x] Develop lap `01M7QEQT1P8CF3SF4D4W8SN7BE` lands work#1440 demand 1+2:
      single-flight guard + manifest-hash rider (vendored petri change) —
      PR #602 merged 2026-11-18 14:40 UTC (`1d979693d0`), reviewer Approved
- [x] Same change lands demand 3: mint-time LRU ceiling (keep last ~6 / ~60 GB
      env-image bytes) — in the same PR, daemon-side `evict_lru_images`,
      feature `env-image-evict` enabled in fabro's graph
- [x] Vendored change exported to the petri fork (ADR-0034/ADR-0027), PINS
      updated — done 2026-11-18 ~14:50 UTC: `denkhaus/petri#3aa8f41915f1`
      and `denkhaus/sandbox-driver#035ab83a4b2a` carry the change (verified
      via ls-tree of both fork commits + `just vendor-pins` green with REAL
      tree_sha verification now that the managed clone exists). NOTE: the
      run's painpoint claiming the export died was stale — its PINS update
      had only rewritten content_sha256/files against the OLD fork_sha,
      which made the fail-open checks read green. `just vendor import` is
      now SAFE.

Host/ops:

- [ ] Deploy the fix: merge gets #1440 onto trunk, but the cook executor
      (`ensure_environment_image` + guard + ceiling) compiles INTO the
      fabro-server binary (vendor/petri is in-tree via the patch blocks) —
      it is NOT live until the next server image build + ship to netcup-5 +
      container restart. Until then the running binary (fork.40, built
      2026-11-17 10:25 CET) still races concurrent cooks and mints without a
      ceiling. Deploy checklist: build the release image, ship, restart,
      verify `fabro version --server` shows the new build, then confirm at
      the next new-tip cook that concurrent gears yield ONE build and the
      petri-env tag count drops to the ceiling on the first eviction sweep
- [ ] Activate the work#168 retention arm on the deploy host (seed
      fabro-tofu-1055: `terminal_sandbox_age_hours 48` +
      `disk_brake_percent 85` + container restart) — relief step 5 of
      work#1425, still owed after yesterday's emergency purge
- [ ] Record before/after `df -h /var/lib/docker` in work#1425 on activation

Adjacent lines (own carriers, own lanes):

- [ ] work#1335 (loop): release-binary fetch in `.fabro/Dockerfile.toolchain`
      + `just host-release` — collapses the per-tip cook cost; makes aggressive
      eviction safe
- [ ] #1188 (loop): df threshold alarm to Slack + the hourly backstop
      sweep executor — IMPLEMENTED in fabro-tofu (`main` @ `ec4c938`):
      GC alert arm (WARN once per crossing / CRIT every run, out-of-band
      webhook, host cron, no server in the loop) + 5 ladder test pins.
      Owed by the user: `gopass insert fabro-tofu/gc-slack-webhook-url`
      (webhook first), then the tofu apply — until both, alerts skip
      silently
- [ ] work#1425 (loop): close after relief + activation are recorded with
      before/after evidence

Verification once #1440 lands:

- [ ] Acceptance: concurrent first cooks of the same tag yield EXACTLY ONE
      full build; concurrent gears complete Start in ~the single-cook duration
- [ ] Acceptance: the petri-env population stays bounded by the mint-time
      ceiling; a 24h-without-purge window no longer reaches disk-critical
      levels; host GC log shows the ceiling holding (tags bounded, GC still
      sweeping residue)
- [ ] Policy-debt follow-ups from the same morning stay on their own lines:
      work#1434 (radius backoff), work#1435 (fire holdover), work#1436
      (admission gate), work#1437 (ps skew degradation), work#1438 (watchdog
      lap latency), work#1439 (output-contract aliases)

Optional, no overlap:

- [ ] work#1272 (BuildKit routing) and work#1208 (layer order) — evaluate only
      after #1440 + #1335 land; they optimize the same cook from different
      angles
