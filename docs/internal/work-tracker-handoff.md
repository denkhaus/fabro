# Handoff: central work-item tracker — deploy done, Phase 2 blocked on GitHub App, Phase 3 started

Written 2026-10-21 (00:10 CEST) on **v1000** by the follow-up session.
Audience: the next host-side agent session. Chat language with the user:
German. All user-facing times: Europe/Berlin.

Authority: `docs/lab/adr/0032-central-work-item-tracker.md` and
`docs/internal/central-work-tracker-migration.md` (the plan). This document
is the session handoff: what landed, what is verified, what is external,
and the exact next actions. It supersedes the 2026-10-20 handoff (seed
`fabro-a1ae`, closed).

## Situation

- The autonomous line stays **OFF** (loop/revisor/architect/conductor
  schedules off). Do not re-arm before the plan's Phase 6 checklist + user
  GO.
- **Push gate is clean and everything is pushed.** Branch `denkhaus`,
  `git log` top: `4d57ff947` (fabro_work_list tool), `e4eebc2af`
  (scripts/up.nu nu-0.115 flock fix), `0601e9162` (previous handoff).
  `origin/denkhaus` matches HEAD. 0 open PRs on `denkhaus/fabro`.
- **The new build is deployed locally** via `just up` (Smoke passed):
  `fabro-lokal` on `127.0.0.1:32276` now serves the Phase-1 API
  (`/api/v1/projects/*` answers 401-unauth instead of 404). The image is
  `ghcr.io/fabro-sh/fabro:local`.
- **Project `fabro` is configured** against the live local server:
  `PUT /api/v1/projects/fabro` → github / `denkhaus/work` /
  `project:fabro` / lane labels loop,develop,revisor,architect,conductor /
  `fabro:run:` / `backpressureOpenPrCap=0`. Stored in the
  `fabro_fabro-storage` volume's SQLite.
- **Phase 2 is blocked on one user action:** the GitHub App (app_id
  `4924979`, client_id `Iv23liLEOEG8fRGFA30v`) is NOT installed on
  `denkhaus/work`. The first live tracker call proved it:
  `503 tracker provider is not available: minting GitHub installation
  access token: GitHub App is not installed for denkhaus, or its
  installation does not include denkhaus/work`.
- **Phase 3 has started:** the read gap (`seeds list`/`seeds ready`/
  `seeds search` had no `fabro_work_*` equivalent) is closed with a new
  read-only `fabro_work_list` run tool. The prompt/script rewrite itself
  has NOT started.

## What landed this session

- **`scripts/up.nu` fix** (`e4eebc2af`): nu 0.115 rejects `(path self)`
  at run time ("can't run after parse-time"), which broke the flock
  re-exec entry of `just up` (exit 127 on every run). Bound as a
  parse-time `const SELF = (path self)`.
- **`fabro_work_list` run tool** (`4d57ff947`, ADR-0032): lists the
  project's OPEN work items read-only (optional `lane`, `limit`
  1..=100 default 50, sorted by priority then recency).
  - `lib/components/fabro-tool/src/work.rs`: `FabroWorkListParams`,
    `ValidatedWorkList` (blank project/lane → teaching error; limit
    clamped), `work_list`, `work_list_text` (count line + one line per
    item).
  - `lib/components/fabro-tool/src/common.rs`: catalog entry +
    `FabroToolBackend::work_list` default refusal.
  - `lib/components/fabro-tool/src/fabro_client.rs` +
    `lib/foundation/fabro-client/src/client.rs`: `list_work_items`
    grew a `limit` argument.
  - `lib/components/fabro-workflow/src/run_tools.rs`: dispatch arm +
    httpmock test pinning `lane`/`limit` on the wire.
- **Live config**: the `fabro` project row (see Situation).

## Verified state (at `4d57ff947`)

- `cargo check --workspace` clean.
- `cargo +nightly-2026-04-14 fmt --check --all` clean.
- `cargo +nightly-2026-04-14 clippy -p fabro-tool -p fabro-workflow -p
  fabro-client --all-targets -- -D warnings` clean.
- nextest: seven-crate suite (fabro-work, fabro-store, fabro-github,
  fabro-tool, fabro-client, fabro-workflow, fabro-types)
  **799/799** (7 skipped); **fabro-server it 1140/1140** (1 skipped).
- `just up` full pipeline green (build → image → compose → health →
  smoke).
- Auth for curl/CI: the local server uses `dev-token` (methods
  `["dev-token"]`); token in `~/.fabro/auth.json` for
  `http://127.0.0.1:32276`, pass as `Authorization: Bearer fabro_dev_...`.
- Vault already carries `GITHUB_APP_PRIVATE_KEY`, `GITHUB_APP_CLIENT_SECRET`
  (`fabro secret list`); settings carry
  `[server.integrations.github] strategy="app" app_id="4924979"
  client_id="Iv23liLEOEG8fRGFA30v"`. The `slug` key is NOT set — set it
  so `/api/v1/system/integrations` can render the install URL.
- Pre-existing, NOT ours: 19 fabro-petri failures (`fabro-ae49`).

## Next actions, in order

1. **USER ACTION (blocks Phase 2):** install the GitHub App on
   `denkhaus/work` (needs `issues: write`, `metadata: read`; Projects
   only if a board is wanted), or add `denkhaus/work` to the existing
   installation under *Settings → Installed repositories*. Then set
   `server.integrations.github.slug` in the server settings so the
   install URL surfaces in the integrations metadata.
2. **Phase 2 live verification** (no line) against the local server:
   - `GET /api/v1/projects/fabro/work-items` must return issue #1
     (`project:fabro-dryrun`) instead of 503.
   - `POST /api/v1/projects/fabro/next` with a REAL non-terminal run id
     and lane (`{"runId":"...","lane":"loop"}`) → outcome `granted`,
     then verify the `fabro:run:<id>` label appears on issue #1.
   - `POST /api/v1/projects/fabro/claims/{id}/release` → label removed,
     label object GC'd (reconciler pass ≤30s).
   - `POST /api/v1/projects/fabro/work-items` create + close a
     throwaway item; confirm the reconciler no-ops on a healthy ledger
     (`docker logs fabro-lokal`).
   - Record results in the plan's Phase 6 matrix.
   - NOTE: a terminal run's claim gets released by the reconciler within
     ~30s (30-min orphan grace applies only to MISSING runs) — do not
     pick a succeeded/failed run for the label-appearance check.
3. **Phase 3 workflow rewrite** (the big one; one-unit rule per
   workflow). The inventory at `4d57ff947`:
   - **`gh` mentions (7 files):** `.fabro/scripts/push-gate.nu`
     (open-PR check → the lane's queue/PR view), develop
     `prompts/planner.md`, `prompts/reviewer.md`, `scripts/closeout.nu`
     (comment only), revisor `prompts/file.md` (ADR-0019 rule text —
     keep the rule, drop the CLI name), pr-simplify `prompts/simplify.md`,
     patch-cves `prompts/patch-cves.md`.
   - **`seeds` call sites:** develop (planner, implementer, reviewer,
     project-facts, tracker-commands-*, scripts planner-preflight,
     claim-check, tracker-guard, closeout, closeout-smoke, gate-bounce,
     evidence), loop (planner, implementer, project-facts), revisor
     (file, analyze, facts, scripts closeout-race, overflow-ledger
     fixtures), architect (file, survey, analyze, facts, workflow.fabro),
     conductor (revise-leg), shared scripts (dup-run-check,
     close-claim-check, salvage-sweep, friction-score, prompt-lint,
     planner-preflight fixtures).
   - **Mapping:** `seeds ready`/`seeds list`/`seeds search` →
     `fabro_work_list` (+`fabro_work_show` for bodies); the atomic
     candidate grant stays `fabro_work_next`; `seeds show <id>` →
     `fabro_work_show <itemKey>`; `seeds create` → `fabro_work_create`;
     `seeds close` → `fabro_work_close` (reason lands as comment);
     misroute/stale-basis → `fabro_work_release`; planner candidate
     source becomes the tool grant, not `seeds ready`. Host-side
     scripts that shell out to `gh` or parse `.seeds` move onto the
     project-scoped API or retire. Dup-run-check's history half is
     unaffected (git only); its tracker-authoritative arm needs the API.
   - Node allowlists: add the needed `fabro_work_*` names to each
     node's `x.fabro_tools` in the same change (ADR-0008).
   - Then: `nu .fabro/scripts/prompt-lint.nu`, `just validate-workflows`,
     targeted tests. The line stays off; land host-side.
4. **Phase 4 data migration**: idempotent importer (open first; closed
   with `Original-Created`/`Original-Closed`), `Legacy-ID: fabro-xxxx`,
   mapping file `docs/internal/seed-id-map.json`, archive JSONL into
   `denkhaus/work/archive/`, parity N/N at freeze (941 records at
   2026-10-21 — RECOUNT at freeze), 0 duplicate ids.
5. **Phase 5 cutover**: freeze `.seeds`, remove seeds from the line's
   toolchain/prompts, gh-free check, update skills (`iterate`,
   `integrate`) and `AGENTS.md`/`CLAUDE.md`.
6. **Phase 6**: verification matrix + crash/recovery drill + one manual
   develop and one manual revisor run; then the re-arm checklist and
   user GO. Re-arm revisor + loop only.

## Do not

- Do not re-arm schedules; the line stays off until Phase 6 + GO.
- Do not re-add `gh` to `.fabro/Dockerfile.toolchain` (ADR-0019 pin) and
  do not give agents a forge token: writes stay engine-mediated.
- Do not use a boolean claim label or a claim comment: the projection is
  the run-id label `fabro:run:<id>`; the DB is the authority.
- Do not hard-code `denkhaus/work`, `fabro`, or GitHub into the core.
- Do not rewrite the already-public `.seeds` git history in this
  migration (separate decision).
- Do not edit `.fabro/cook/**` — it is a mirror, not a source.

## Operational facts

- Host: **v1000**. Local server: `http://127.0.0.1:32276` (container
  `fabro-lokal`, image `ghcr.io/fabro-sh/fabro:local`, volume
  `fabro_fabro-storage`). Production: `FABRO_SERVER=https://mirtuell.net`
  still runs the OLD build — the Phase-1 API does not exist there yet.
- Deploy: `export XDG_RUNTIME_DIR=$HOME/.cache/just-run && just up`
  (flock re-exec fixed this session).
- Dev token: `~/.fabro/auth.json` → `fabro_dev_ff1c...add6` for
  `http://127.0.0.1:32276` (server auth method `dev-token`).
- `gh` auth works as `denkhaus`. Tracker repo: `denkhaus/work`
  (private); dry-run probe: issue #1 (`project:fabro-dryrun`).
- Seeds CLI present (`seeds` v0.9.0) but READ from here on — the line's
  writes move to `fabro_work_*` in Phase 3/5; only this handoff's seed
  bookkeeping may still write `.seeds` until Phase 5.
- Commands used for verification:
  - `cargo +nightly-2026-04-14 fmt --check --all`
  - `cargo +nightly-2026-04-14 clippy -p fabro-tool -p fabro-workflow -p fabro-client --all-targets -- -D warnings`
  - `cargo nextest run -p fabro-work -p fabro-store -p fabro-github -p fabro-tool -p fabro-client -p fabro-workflow -p fabro-types --no-fail-fast`
  - `cargo nextest run -p fabro-server --features test-support --no-fail-fast`
