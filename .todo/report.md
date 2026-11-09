# Session report — the 24h-outage restore window (2026-11-09/10)

Transport artifact for the NEXT session (ADR-0036). It supersedes
`.todo/HANDOFF-DEPLOY-WINDOW.md` (deleted in the same change; its
remaining items 3f/3g are folded into §6 below). Delete this file when
its demands are consumed.

## 1. State at handoff

- **mirtuell.net is LIVE again**: `{"status":"ok"}`, image
  `denkhaus/fabro:0.362.0-fork.32-6ee9d8b22846` (host-built, ADR-0041),
  deployed ~22:52 Berlin 2026-11-09 after 24h dark by decision.
- **All four gears unpaused and firing** — but EVERY fire 422s at run
  create (see §2). The line is up but produces no runs. Nothing is
  corrupted; each tick fails cleanly and the breaker stays at 0.
- Deploy gate was composed green before apply: full 39-probe table
  (bench 1109-20xxxx) + targeted probe-42 + mini + engine_pr-block +
  nu-lockstick inputs.
- fabro tip: `56cfc2b84` (all pushed). fabro-tofu main: `de25af0`.
  fabro-test main: `bb4bf3e`.

## 2. THE BLOCKER — the `{ path }` dockerfile form (open decision A/B)

The admission body (obtained by replaying the scheduler's intent):

```
fabro create --server https://mirtuell.net --workflow-repo denkhaus/fabro \
  --workflow-ref denkhaus develop --target denkhaus/fabro@denkhaus \
  --environment toolchain
→ run intent could not be compiled: Validation failed:
  unsupported.workflow_toml.environments.image.dockerfile:
  `[environments.toolchain.image] dockerfile` uses the `{ path }` form,
  which is Fabro's persist-time inlining; this runner reads no files here
  [run_compile_invalid]
```

Root cause chain (all verified, see §3 for how):

1. ADR-0042 made `image.dockerfile` LIVE in petri (vendored
   `vendor/petri/crates/fabro/frontend/src/workflow_toml.rs`):
   **inline text only**. The `{ path }` form is FABRO's to resolve at
   persist time (version assembly) — "persist-time inlining".
2. The four line workflows author `{ path = "../../Dockerfile.toolchain" }`
   (correct per ADR-0042's authoring intent).
3. Fabro's lowering resolves `{ path }` for its OWN in-memory settings
   layers (`run_compiler::resolve_dockerfile`) — but petri re-parses the
   RAW layer tomls from the bundle (`workflow.toml` AND
   `.fabro/project.toml`, per FORMAT.md "Both tables are read from every
   layer") and refuses the raw `{ path }`.
4. The A-path E2E proof used inline-text fixtures; the `{ path }` authoring
   form never drove a real create until the gears fired tonight.
5. Same class in the PROJECT layer (vestigial `[environments.fabro-dev.image]`,
   the retired VNC smoke sandbox) — FIXED in `56cfc2b84`. The workflow-layer
   instances remain.

**THE FIX (proper)**: persist-time inlining in the collector — rewrite
`dockerfile = { path = … }` → `{ text = <file content> }` in the stored
config sources (workflow config AND the project toml entry in the files
map) at bundle assembly. Seam: `workflow_bundler.rs`
`collect_workflow_entry` (config.source at ~line 122-141) and the
project-config site (~line 61-68); the dockerfile file content is already
collected into `files` by `collect_environment_dockerfile`. Use a generic
`toml::Value` roundtrip (NOT SettingsLayer — it would drop unknown keys).
Red-first pin: the collector test fixture (`write_complete_fixture`,
`path = "Dockerfile"`) asserts the stored source carries
`dockerfile = { text = "FROM alpine\n" }`.

**DECISION PENDING (user, next session):**
- **A (recommended)**: the proper collector fix as sprint work; gears 422
  harmlessly until it lands.
- **B**: hot-unblock tonight-style by inlining the ~300-line Dockerfile
  text into the four workflow.tomls (works immediately; 4× duplication
  that drifts on every toolchain edit — the smell persist-inlining exists
  to avoid).

## 3. Evidence trail for §2 (what was ruled OUT, with proof)

- **NOT the dockerfile path**: fixed first (`5f75bd257` — the four tomls
  had one `.fabro` too many; the 20:30 UTC adjudicator tick's packaging
  error `.fabro/.fabro/Dockerfile.toolchain` disappeared after).
- **NOT credentials**: captured the server's live git header on the host
  (proc capture, token shredded after) and replayed it — the fetch of the
  fix commit succeeds. A deliberately broken token 400s (control).
- **NOT the shallow cache**: the automation-repo bare cache was rebuilt
  shallow during the 11-08 chaos; every fire paid a full-history
  `git fetch --unshallow`. The 21:30 UTC fire's unshallow COMPLETED
  (782 MB, `shallow` file gone); later ticks still 422ed.
- **NOT model/window readiness**: zai models report configured=true on
  production; usage empty.
- **NOT the sandbox provider policy for the gears' intents** (toolchain
  env = docker): that check fires only for the DEFAULT environment
  (project.toml pins `id = "fabro-dev"`, provider daytona, disabled on
  prod — pre-existing; surfaced via `fabro preflight`, which cannot pass
  `--environment`).
- The 422 body is discarded by the scheduler (`Failed to create scheduled
  automation run … status=422`, WARN, no body) and the admission logs only
  `error_chain=["Validation failed"]` — the diagnostics gap that cost
  hours. `fabro create --workflow-repo` is the reproduction command.

## 4. What landed this session (chronological, all pushed)

**fabro (denkhaus):**
- `13c9667bc` tests: observation-list mock carries fork.32 required keys
  (the handoff's 3e — the tester-gate red).
- `226dc6225` justfile: host-release ssh arg-joining + fmt (the empty
  `--sha ''` died at nu's parser).
- `138b222fb` lint-nu: hyphen arm for prose parens — 8 latent crashes
  escaped (push-gate, run_workflow, reporter-smoke×2, …); RED/GREEN
  proven; def-awareness for real command calls.
- `c41606f9c` host-release: nu interpolation semantics (`\($VAR)` is
  fully literal!) — six messages + the rev-parse caret bug.
- `8fb33ffb9` smoke: ps roundtrip targets the local stack (the CLI
  default is mirtuell.net since 2026-11-07; the bare `ps` probed the dark
  production).
- `f97b50dd0` host-release: `sh -c` not `-lc` (login shell resets PATH,
  cargo not found).
- `3e19bc47b` host-release: clone mounts at its REAL host path (nested
  docker resolves bind sources on the host daemon) + **NU_VERSION
  0.116.1** (vendored hook nu lockstep, official musl sha256s).
- `954a156c9` justfile: ssh keepalive (a silent network drop zombied a
  90-minute build).
- `8c0162382` + `6ee9d8b22` release-builder: buildx CLI plugin INSIDE the
  builder (client-side; the host package alone changed nothing) — tag
  `docker/buildx-bin:0.38.2` (bare 0.38 doesn't exist).
- `5f75bd257` workflows: toolchain dockerfile path `../../Dockerfile.toolchain`.
- `56cfc2b84` project.toml: drop the vestigial fabro-dev image block.

**fabro-tofu (main):**
- `5b1df48` docker-host `release_install` (git + /opt/fabro-release; tofu-
  provisioned, not ad-hoc SSH).
- `44da7c0` docker_config_heal (the 2026-11-08 credsStore damage) + fixture
  test; **RETIRED same day by user decision** (`de25af0`) after the repair.
- `4493d3c` buildx plugin install; `772befa` BOTH resources re-landed as
  **nu scripts** (user caught my inline-sh violation of ADR-0015);
  docker_install left as historical sh with a note.
- `4b1…/live pin`: **variables.tf default now names the LIVE image**
  `:0.362.0-fork.32-6ee9d8b22846` (the committed-default rule).

**fabro-test (main, PRs #142/#143/#144/#146):**
- nu pin 0.116.1 (was 0.115.0 — user catch).
- **probe-42-observation-links**: the work-item observation link lifecycle
  (reverse join on the wire, dedupe at the edge, live-link close block,
  unlink→radius-free close). First drive green.
- platform row: server container nu == harness pin.
- two nu-0.116 fixes in the bench itself (leading-`and` lines;
  `mut x: any` for the null-sentinel pattern).

## 5. Work items filed (tracker)

- **denkhaus/work#1230** — the outage record (retroactive, emergency
  clause): timeline, the rehearsal's ten defects, follow-ups.
- **denkhaus/work#1231** — ghcr mirror denied: the app installation lacks
  organization package write (mirror push failed post-deploy; fail-open
  by design; the variables.tf fallback note references this).
- **denkhaus/work#1232** — the inert server-managed `toolchain`
  environment entry cannot be deleted (409: the gears reference it) —
  placement-semantics question post-ADR-0042.

**Still to file (next session):** the `{ path }` dockerfile defect itself
(§2 — the P1), + the three diagnostics gaps: scheduler discards the
create-response body; admission logs no diagnostics; manual-vs-scheduled
compile asymmetry (`fabro run`/`create` refuse `{ path }` locally while
the server's automation path packages fine).

## 6. Restart-list remainder (from the old handoff)

- 3a health ✓ / 3b unpause ✓ / 3c outage item ✓ (#1230) / 3d hygiene:
  409-blocked, filed #1232 / 3e RED test ✓ (`13c9667bc`).
- **3f live proofs — BLOCKED on §2**: the first develop fire must build
  the toolchain image at run init on the host daemon and gate GREEN
  (nu 0.116.1 + regen; closes #1177/#1183); watchdog pass title verifies
  the titling chain (d52bff59f/#513).
- **3g**: retry/fork #1186's stranded run (server-side, after §2).
- The sandbox-driver sha for this image:
  `629223a0c7553bd3dcaeadd26e4b46727168d45c1b366e4d7ec804b6c8ccd6dc`
  (used in the live apply via -var; consider committing as default).

## 7. The UI grill — decisions captured, work NOT started

User answered all seven (2026-11-09, chat):
1. Width: **all three tables max-w-[96rem]** (the run tab loses wide:true).
2. Background: **panel #252C3D** for all three tables.
3. Tracker links in observation tables: **general approach** — the work
   item is an abstract interface (forgejo/gitea/gitlab possible; only
   github implemented) → server-side URL resolution (API field on
   ObservationWorkItemLink), NOT client-side github hardcoding.
4. Work-table key column: the internal detail link is **BROKEN** (a work
   item detail page never renders) — investigate/see it first; external
   direct-linking makes no sense until then.
5. Observation date format: **absolute "11.12.2026 11:38:44"**
   (DD.MM.YYYY HH:MM:SS), NOT relative.
6. Pager label **"/ page" removed entirely** (selector + aria-label only).
7. Run id: **never abbreviated** (the feed slices to 13 chars today).

## 8. Environment notes for the next session

- Local stack: rebuilt at tip `3e19bc47b` (container nu 0.116.1 — the
  lockstick row's inputs verified). The local automation-repo cache's
  origin was manually pointed at github.com (was a stale local path).
- Production OAuth access_token is SHORT-LIVED: raw curls need a CLI call
  first to refresh it (the fabro-test auth-header comment documents the
  same trap). Token is `access_token`, not `token`.
- `fabro automations --server X list` (flag before the subcommand);
  `fabro run <wf>` takes `--server` AFTER the verb.
- Host artifacts of the diagnosis are cleaned (captures shredded,
  strace/tcpdump gone; tcpdump was never installed — strace exists).
- The ghcr mirror apply error left `null_resource.ghcr_mirror` uncreated
  in state (its push failed after the container was healthy); the next
  plain apply retries it — disable with `-var ghcr_mirror_enabled=false`
  until #1231 is fixed, or fix the installation permission first.
- Watchdog ticks hourly, adjudicator */15, merger :10/:40, develop :20/:50
  (all Berlin = UTC+1). Every fire currently 422s cleanly.
