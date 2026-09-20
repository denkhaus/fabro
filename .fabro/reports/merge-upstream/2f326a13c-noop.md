# Upstream merge report — 2f326a13c (no-op pass)

**Date:** 2026-09-08 · **Branch:** `fabro/run/01M20N60ZY6DZDTX60WW0161KP`
(aligned with `origin/denkhaus` at `9613ec4d8`)

**Outcome:** no-op — `upstream/main` is already fully contained in
`origin/denkhaus`. Nothing merged, no merge commit created, tree
byte-identical to pre-merge HEAD. Per the uniform filename contract this
report overwrites the previous `2f326a13c-noop.md` (written at base
`cb805bc65`; git history keeps the prior version). The real merge of this
upstream head is covered by `2f326a13c.md` (2026-09-06).

## Merge identity

- Range: `origin/denkhaus..upstream/main` = **0 commits**; upstream head
  `2f326a13c4c04e5f655d8d7c35a065653a411999` (`Bump version to
  0.348.0-nightly.0`) — unchanged since the 2026-09-06 merge.
- Containment proof: `git merge-base HEAD upstream/main` = `2f326a13c`
  (equals the upstream tip); `git rev-list --count upstream/main..HEAD` =
  550 fork commits on top.
- `git merge upstream/main --no-commit` → `Already up to date.` (exit 0).

### Environment pitfall (recurring, process note)

The run sandbox clone arrived **shallow** again (boundary `43036bb477`),
and the first `git rev-list --count origin/denkhaus..upstream/main`
reported a misleading **4919** with "no common ancestor" from
`git merge-base` and a failed `--is-ancestor` check — the exact trap the
prior no-op report documented. `git fetch --unshallow upstream` completed
the repository; the true count is 0. This is the **third** consecutive
run to hit the trap (previous boundaries `661fa2a1e`, `9cf304339`).
Mergers: run `git rev-parse --is-shallow-repository` before trusting any
range math in this environment.

## Conflicts resolved (by class)

None — no merge content. No conflict classes seen. Adapted fork call
sites: **none** (empty merge diff).

## Verification commands + results

| Command | Result |
|---|---|
| `git remote add upstream …` + `git fetch upstream --prune` + `git fetch origin` | green |
| `git fetch --unshallow upstream` | green (repo complete; range math reliable) |
| `git rev-list --count origin/denkhaus..upstream/main` | **0** |
| `git merge-base HEAD upstream/main` | `2f326a13c` (= upstream tip, contained) |
| `git merge upstream/main --no-commit` | `Already up to date.` (exit 0) |
| `just qualitygate` | **GATE GREEN** (exit 0): run branch pre-checkpoint → base ungrounded → working-tree diff has no `lib/` paths → gate degraded to `cargo +nightly-2026-04-14 fmt --check --all` per `scripts/qualitygate.nu` design ("no crates touched", "format clean"). Duration ≈ 5 s. |
| `cargo nextest run -p fabro-workflow -E 'test(pipeline::finalize)'` | 26/26 (publish-blocked + boundary taxonomy) |
| `cargo nextest run -p fabro-workflow -E 'test(seed_cycles)'` | 4/4 (fabro-45d0 re-attachment intact) |
| `cargo nextest run -p fabro-server -E 'test(publish_blocked)'` | 2/2 (Slack lifecycle remediation intact) |
| `cargo nextest run -p fabro-github` | 92/92 (PR create retry crate) |
| `cargo nextest run -p fabro-workflow -E 'test(pull_request)'` | 55/55 (PR pipeline incl. retry path) |

Gate-scope note: with a zero-commit upstream delta there is no merge diff
for clippy/nextest to newly break; the current tree's crate-level gates
were carried by the runs that landed each fork commit since 2026-09-06,
and the 2026-09-06 merge itself ran the full gate (see `2f326a13c.md`).
Unlike the prior no-op pass (grep-only evidence), this pass re-ran the
touchpoint test probes listed above against a fresh build.

## Feature-regression analysis (touchpoint walk)

Tree is byte-identical to `origin/denkhaus`, so regression through this
pass is impossible by construction; every touchpoint was re-probed with
fresh symbol + test evidence at the current HEAD.

| Feature (seed) | Status | Evidence in tree at `9613ec4d8` |
|---|---|---|
| Publish-blocked taxonomy (fabro-67e5, closed) | ✅ present | `SuccessReason` ×9 in `lib/foundation/fabro-types/src/status.rs`; `publish_blocked_status()` at `lib/apps/fabro-server/src/server.rs:1037`, routing at `:4421`; probe 26/26 finalize + 2/2 publish_blocked |
| Boundary exit kind (fabro-08b4, closed) | ✅ present | `build_terminal_event`/`apply_boundary_upgrade` ×14 matches in `lib/components/fabro-workflow/src/pipeline/finalize.rs` (covered by the same 26/26 probe) |
| PR create retry (fabro-67e5, closed) | ✅ present | `create_pull_request_with_attempts` defined at `lib/components/fabro-workflow/src/pipeline/pull_request.rs:667`, call site `:650`; probes 92/92 + 55/55 |
| PR model plumbing (fabro-890b, closed) | ✅ present | `fn resolve_pr_model` at `lib/components/fabro-workflow/src/operations/start.rs:849` |
| spa_refresh mirror race (fabro-332e, OPEN) | ✅ present | `lib/foundation/fabro-dev/src/commands/spa_refresh.rs`, `scripts/smoke.nu`, `scripts/wait-healthy.nu`; `just smoke` is deploy-side, out of scope here |
| ask duplication (fabro-bd6c, OPEN) | ✅ present | `fn render_event` at `lib/apps/fabro-cli/src/commands/run/ask.rs:71`; UX seed still open, code intact |
| attach replay indistinguishable (fabro-204e, OPEN) | ✅ present | `replay_run_with_client` at `lib/apps/fabro-cli/src/commands/run/attach.rs:227` (call site `:192`) |
| Preamble aggregate budget (fabro-a85b, OPEN) | ✅ present | `demote_large_values_for_prompt` ×11 in `lib/components/fabro-workflow/src/artifact.rs` (incl. tests) |
| just-up lock + smoke (landed) | ✅ present | `scripts/smoke.nu`, `scripts/wait-healthy.nu`, justfile recipes |
| run_workflow.nu pipeline (landed) | ✅ present | `scripts/run_workflow.nu` |
| Auto-merge wiring (fabro-ab2c, closed) | ✅ present | `.github/workflows/dogfood-gate.yml` + branch protection (engine-owned) |
| seed_cycles (fabro-45d0, hotspot) | ✅ present | `context::update_seed_cycles` at `lib/components/fabro-workflow/src/lifecycle/mod.rs:404`; probe 4/4 |

**Adapted fork call sites this pass: none** (empty merge diff).

Fork-surface note for the next real merge (carried forward): since the
2026-09-06 merge the fork added server-side admission work that upstream
will meet for the first time — docker environments with inline-only
dockerfiles for git intents (fabro-0253, `66043eab`), the
GitHub-integration gate before credential lookup (fabro-c274,
`9d1ae616`), and the ADR-0019.6 principal capability gate line
(fabro-e505). Expect `lib/apps/fabro-server` and
`lib/components/fabro-workflow` admission paths to become conflict
hotspots when upstream moves.

## Obsolescence notes

- Upstream has not moved since 2026-09-06 — no seeds or fork features
  superseded this pass; nothing to file or update.
- Standing watchlist (from `references/touchpoints.md`) unchanged:
  SQLite read-model line (`AppState::load_run_projection`), RunIntent
  admission matrix (branch/tag/SHA), node-on-failure exclusion for
  `develop`, fabro-b7c4 legacy-catalog fix still un-offered-upstream.

## What it means for us

- No new upstream themes this pass — no per-theme lines apply.

## Workflow changes

None: the merge delta is empty, so `develop` and its scripts
(`scripts/run_workflow.nu`, `scripts/qualitygate.nu`) need no adjustment;
the next real upstream move is the trigger to re-evaluate.
