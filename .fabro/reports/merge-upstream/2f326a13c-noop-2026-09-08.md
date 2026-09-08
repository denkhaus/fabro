# Upstream merge report — 2f326a13c (no-op verification pass)

**Date:** 2026-09-08 · **Branch:** `fabro/run/01M1ZVH0B5V5RHRCY393T24GXG`
(aligned with `origin/denkhaus` at `aaa431e5c`)

**Outcome:** no-op — `upstream/main` is already fully contained in
`origin/denkhaus`. Nothing to merge, no merge commit created. This file is
named with a `-noop-2026-09-08` suffix to preserve the real merge report
`2f326a13c.md` (2026-09-06), which covers the merge that brought this exact
upstream head in.

## Merge identity

- Range: `origin/denkhaus..upstream/main` = **0 commits**; upstream head
  `2f326a13c4c04e5f655d8d7c35a065653a411999` (`Bump version to
  0.348.0-nightly.0`, 2026-09-06T09:29:10Z) — newest upstream ref of any
  kind (branches checked).
- Proof of containment: `git merge-base --is-ancestor upstream/main
  origin/denkhaus` → exit 0; merge-base equals `2f326a13c` itself.
- `git merge upstream/main --no-commit` → `Already up to date.` — zero
  conflicts, zero tree delta (`git status` clean except this run's journal).

### Environment pitfall (process note, not a merge issue)

The run sandbox clone arrived **shallow** (boundary `661fa2a1e`, "seeds:
sync 2026-09-06"), so the first `git rev-list --count
origin/denkhaus..upstream/main` reported a misleading **4919** — the
denkhaus side of the shared ancestry was cut off, not actually diverged.
`git fetch origin --deepen=300` restored enough history for merge-base
resolution; the true count is 0. Any merger seeing a huge sudden range in
this environment should check `git rev-parse --is-shallow-repository`
before concluding upstream moved.

## Conflicts resolved (by class)

None — no merge content. No conflict classes seen this pass. No fork call
sites required adaptation (adapted call sites: **none**; empty merge diff
means the tree is byte-identical to pre-merge `origin/denkhaus`).

## Verification commands + results

| Command | Result |
|---|---|
| `git fetch upstream --prune` + `git fetch origin` (+ `git fetch origin --deepen=300`) | green |
| `git merge-base --is-ancestor upstream/main origin/denkhaus` | exit 0 (contained) |
| `git merge upstream/main --no-commit` | `Already up to date.` |
| `just qualitygate` | **GATE GREEN** (exit 0): merge diff empty → touched-crates = ∅ → gate degraded to `cargo +nightly-2026-04-14 fmt --check --all` per `scripts/qualitygate.nu` design ("no crates touched", "format clean"). Duration ≈ 6 s. |

Gate-scope note: with a zero-file merge delta there is no code change that
clippy/nextest could newly break; the current tree's crate-level gates were
carried by the runs that landed each fork commit since 2026-09-06, and the
2026-09-06 merge itself ran the full gate (see `2f326a13c.md`).

## Feature-regression analysis (touchpoint walk)

Tree is byte-identical to `origin/denkhaus`, so regression through this pass
is impossible by construction; every touchpoint was re-probed anyway.

| Feature (seed) | Status | Evidence in merged tree |
|---|---|---|
| Publish-blocked taxonomy (fabro-67e5, closed) | ✅ present | `SuccessReason` ×9 in `lib/foundation/fabro-types/src/status.rs`; `build_terminal_event` ×9 in `lib/components/fabro-workflow/src/pipeline/finalize.rs`; `publish_blocked` ×4 in `lib/apps/fabro-server/src/server.rs` |
| Boundary exit kind (fabro-08b4, closed) | ✅ present | `apply_boundary_upgrade` at `lib/components/fabro-workflow/src/pipeline/finalize.rs:68`; tests `boundary_upgrade_parks_engine_error_green`, `boundary_upgrade_ignores_other_exit_kinds`, `boundary_exit_kind_reports_boundary_reason`; key `INTERNAL_EXIT_KIND` defined `lib/foundation/fabro-types/src/context_keys.rs:55`, used `finalize.rs:584/1134`, `lifecycle/mod.rs:435` |
| PR create retry (fabro-67e5, closed) | ✅ present | `create_pull_request_with_attempts` `lib/components/fabro-workflow/src/pipeline/pull_request.rs:667` (prod call :650; tests :2397/:2420); `CreatePullRequestError` in `lib/components/fabro-github/src/lib.rs` |
| PR model plumbing (fabro-890b, closed) | ✅ present | `resolve_pr_model` ×2 in `lib/components/fabro-workflow/src/operations/start.rs` |
| spa_refresh mirror race (fabro-332e, OPEN) | ✅ present | `lib/foundation/fabro-dev/src/commands/spa_refresh.rs` (+ `spa_check.rs`); smoke = deploy-side (`just smoke`) |
| ask duplication (fabro-bd6c, OPEN) | ✅ present | `render_event` ×2 in `lib/apps/fabro-cli/src/commands/run/ask.rs` |
| attach replay indistinguishable (fabro-204e, OPEN) | ✅ present | attach command at `lib/apps/fabro-cli/src/commands/run/attach.rs` (replay via `replay_run_with_client`, :227) — note: module moved from `cli attach.rs` in earlier history; UX seed still open, code intact |
| Preamble aggregate budget (fabro-a85b, OPEN) | ✅ present | `demote_large_values_for_prompt` ×11 in `lib/components/fabro-workflow/src/artifact.rs` (incl. tests) |
| just-up lock + smoke (landed) | ✅ present | `scripts/smoke.nu`, `scripts/wait-healthy.nu`, justfile recipes |
| run_workflow.nu pipeline (landed) | ✅ present | `scripts/run_workflow.nu` |
| Auto-merge wiring (fabro-ab2c, closed) | ✅ present | `.github/workflows/dogfood-gate.yml` + branch protection (engine-owned) |

**Adapted fork call sites this pass: none** (empty merge diff).

## Obsolescence notes

- Upstream has not moved since 2026-09-06 (`2f326a13c`, v0.348.0-nightly.0)
  — no seeds or fork features superseded this pass.
- Standing watchlist (from `references/touchpoints.md`) unchanged:
  SQLite read-model line (`AppState::load_run_projection`), RunIntent
  admission matrix (branch/tag/SHA), node-on-failure exclusion for
  `develop`, fabro-b7c4 legacy-catalog fix still un-offered-upstream.

## What it means for us

- No new upstream themes this pass — fork stays 417 commits ahead of
  `upstream/main` with full shared ancestry restored (since the 2026-09-06
  merge repaired the PR #30 squash divergence).
- Next merge pass should re-run this procedure when `upstream/main` moves
  past `2f326a13c`; expect the shallow-clone deepen step to be needed again
  in fresh run sandboxes.
