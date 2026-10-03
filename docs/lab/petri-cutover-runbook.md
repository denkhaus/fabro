# Petri Cutover Runbook (fabro-d659, W5)

Status: DRAFT, ready for execution review. Every step is ordered; no
step starts before the previous is green. Production deploys are
USER-ORDER ONLY (standing rule) — this runbook is the checklist the
user and the agent walk together.

## Preconditions

- [ ] Local staging green at the exact commit to deploy: `just smoke`
      8/8, fork suites green (`cargo nextest run -p fabro-workflow
      -p fabro-tool -p fabro-petri -E 'test(fork_)'` + the fork test
      binaries), workspace `cargo build --workspace` clean.
- [ ] No open capability-gate seeds blocking the deploy (fabro-11d9 is
      known-open and user-gated; deploy without it means accepting
      installation-token run-push limits as-is).
- [ ] The seeds repo grammar migration (step 0) is DONE — see below.

## Step 0 — Seeds graph grammar migration (BEFORE any deploy)

The seeds repo's workflow graphs are written for the OLD production
grammar and fail the rebuild validator wholesale (observed on
denkhaus/seeds main: bare `inspects`, node-level
`fabro_tools`/`fs_write`/`skills`/`preamble_stages_ignore`, edge
`kind`, graph `preamble_budget_kb`/`cycle_counter_reset_key` — the
rebuild wants `x.`-prefixed unknown attributes).

- [ ] In ~/dev/seeds: `fabro validate` each of conductor/develop/
      revisor/merge-upstream; x.-prefix every engine-unknown attribute
      the validator names (the analysis doc's attribute-survival matrix
      is the reference).
- [ ] Their line (or the operator, on user order) lands the pass; the
      fabro side never pushes there.
- [ ] Re-run `fabro validate` green for BOTH repos' graphs (fabro's
      five + seeds' four). Without this, the conductor-seeds automation
      dies at workflow packaging on the new engine (seeds-800d class)
      and takes the seeds line down at cutover.

## Step 1 — Era-Check against the prod DB

- [ ] On the prod host: `sqlite3 <storage>/fabro.sqlite3 "PRAGMA
      wal_checkpoint(TRUNCATE);"` THEN copy the file (never cp a live
      WAL database).
- [ ] Run the NEW binary against the snapshot, isolated:
      `--network none`, a dummy `SESSION_SECRET`, the prod
      `settings.toml`. Startup must complete — this proves the
      petri_records/platform_records migrations and the run_events drop
      apply cleanly over real data (the eec6 era-check point: the
      petri engine has no boot-time replay-verify, so this is the one
      explicit compatibility probe).
- [ ] Record the run output (migrations applied, rows) into the
      cutover report.

## Step 2 — Backup

- [ ] Full copy of the prod storage volume (sqlite + blobs + journals),
      named with date and pre-petri marker, retained until the
      post-cutover report is accepted.

## Step 3 — Deploy window

- [ ] `fabro ps --server https://mirtuell.net`: NO conductor/develop/
      revisor pass running (status JSON is {kind: ...}, never
      string-compare). Automations paused if any schedule could fire
      mid-deploy (PUT automation replace, full body, If-Match,
      on_overlap: skip verified after).
- [ ] `just image-release` (ghcr push), then `cd ~/dev/fabro-tofu &&
      TF_DATA_DIR=.terraform-prod tofu apply -var-file=envs/prod.tfvars`
      with the fresh `fabro_image_ref` digest.
- [ ] Server-managed toolchain environment: PUT the new image digest
      (env update), and on the host `docker pull` the fresh toolchain
      image so sandbox creates do not pay first-use pulls (the
      fabro-0e9c registry-auth workaround stays in force).
- [ ] Smoke: health, ps, SPA serves, authenticated automations probe.

## Step 4 — Supervised conductor pass (one)

- [ ] ONE conductor pass on the new engine, supervised live (events
      followed); the pass must run against BOTH graph sets if the
      seeds line is scheduled (their graphs passed step 0).
- [ ] Watch specifically: workflow packaging (grammar), sandbox create
      (plugin checksums, `PETRI_SANDBOX_DOCKER_SHA256` pin form),
      checkpoint commits (the fabro-0c08 guard logs a warn only if
      exhaustion ever hits), PR publish + dogfood gate.

## Step 5 — Post-cutover verification (fabro-d420 addition)

- [ ] Delete a finished run via `DELETE /api/v1/runs/{id}`; the host
      must lose its `fabro-run-*`/`petri-*` containers (petri prune +
      lease ledger path; staging-proven 2026-09-22, evidence in
      fabro-44d8). Retention stays `Always` unless a knob lands.

## Step 6 — Make denkhaus-petri the line

- [ ] Archive `denkhaus` as `archive/denkhaus-pre-petri` (tag, branch
      retired) — the conserved pre-petri world stays recoverable.
- [ ] Move the line: conductor target branch -> denkhaus-petri,
      automations re-pointed, branch protection (dogfood-gate) moved.
- [ ] merge-upstream skill world update: SKILL.md + touchpoints.md
      already carry the petri-era tables; re-check after the branch
      flip.

## Step 7 — Report + reflection

- [ ] Cutover report (steps green, evidence links, costs).
- [ ] Iterate-skill self-reflection: the petri migration lessons land
      as skill edits (this session's are queued in the reflection
      notes); update the friction-score verdict expectations for the
      new engine.

## Rollback

Any red step before Step 4 completes: re-point tofu to the previous
`fabro_image_ref`, restore the storage volume from Step 2, resume the
old line (denkhaus stays intact until Step 6 — that ordering is the
rollback safety). After Step 6 the rollback is the archive tag plus the
volume backup, by design.
