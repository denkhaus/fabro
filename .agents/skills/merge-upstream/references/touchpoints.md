# Feature touchpoints — PETRI ERA (branch denkhaus-petri)

The fork line lives on `denkhaus-petri` (base: upstream/main 40419cbd2,
created 2026-09-20; epic fabro-9930 owns the port waves; full analysis:
`docs/lab/petri-integration-analysis.md`). `denkhaus` is the conserved
pre-petri branch; its old-engine touchpoint rows live in that branch's
history and in the analysis doc — do not resurrect them here.

Merges walk `upstream/main -> denkhaus-petri`. Every durable fork feature
keeps TWO pins: a row here (LLM-walked) AND a fork-only test a merge can
never drop. A row without a test is a gap; a test without a row is
invisible to the merge walk.

## Landed pins (verify after every upstream merge)

| Feature (seed) | Petri anchor | Pin | Fast verification |
|---|---|---|---|
| Automations+env CLI family (fabro-6c16, W1-1) | lib/apps/fabro-cli/src/commands/{automations,env}/; If-Match replace in automations/mod.rs | 12 fork-only it-tests `fabro-cli/tests/it/cmd/{automations_*,env_*}.rs` | `env -u FABRO_SERVER cargo nextest run -p fabro-cli --test it -E 'test(automations) \| test(env_)'` |
| Breaker + overlap model (fabro-a52f, W1-1/-2) | fabro-automation/src/breaker.rs; migrations 2026090201/2026090601 on the petri chain | inline breaker tests in fabro-automation + server suite | `cargo nextest run -p fabro-automation -p fabro-db` |
| Provider gate + breaker exemption (fabro-a52f, W1-2) | server/fork_line_recovery.rs (GateState, cron_fire_allowed, is_quota_park); automation_breaker.rs (terminal_failure reads lifecycle.conclusion_failure) | `mod breaker_exemption_pin` INSIDE server/fork_line_recovery.rs (petri-native seeding: seed_run_row + park_run_with_signature) | `env -u FABRO_SERVER cargo nextest run -p fabro-server -E 'test(quota_parks)'` |
| Publish-blocked taxonomy (fabro-6655, W1-3) | fabro-petri/src/projection/fork_taxonomy.rs; seams in projection/coordinator.rs (success arm) + projection/platform.rs (PullRequestFailed re-classify); SuccessReason::{PublishBlocked,Boundary} in fabro-types | fabro-petri/tests/fork_taxonomy.rs | `cargo nextest run -p fabro-petri --test fork_taxonomy` |
| Taxonomy vocabulary (W1-1 down-payment) | fabro-types status.rs: SuccessReason::{Boundary,PublishBlocked}, FailureReason::{Deadlock,SoftStop,ApprovalTimeout}, BlockedReason::QuotaRateLimit; run_failure::is_quota_rate_limit_failure; RunLifecycle.conclusion_failure (build_summary maps conclusion.failure) | covered by fork_taxonomy pin + fabro-types suite | `cargo nextest run -p fabro-types` |

## Pending ports (wave seeds own the detail)

| Wave | Seed | Feature |
|---|---|---|
| W2 | fabro-a875 | Duplicate-child guard (fabro-tool/create.rs seam survives) |
| W2 | fabro-2889 | Diff-based publish protection (supervisor + platform records) |
| W2 | fabro-b5a9 | PR-create retry + PR-model plumbing |
| W2 | fabro-6945 | Fork catalog overlay (fabro-llm seam) |
| W3 | fabro-2e7b | Quota park on Attractor tiers (ADR-0021 rev 2; BlockedReason::QuotaRateLimit + SoftStop taxonomy already landed, engine-side production is the redesign) |
| W3 | fabro-288d | Exit kinds deadlock/soft on tiers (ADR-0010 rev; FailureReason variants landed; fold docking documented in fork_taxonomy.rs) |
| W3 | fabro-788b / fabro-fa0a | Preamble budget / seed_cycles on Attractor |
| W3 | fabro-1392 / fabro-9b1b / fabro-a044 | Validation rules family / hooks family / workflow transforms (W0 inventory gaps) |
| W3 | fabro-96c6 | Workflow asset rework develop/conductor/merge-upstream (stall_timeout survives; DOT parser reads all 5 graphs — pinned snapshot fabro-dot) |
| W3 | fabro-aa5f | Stage envelope (ADR-0009 rev): x.fs_write/x.fs_hide + x.preamble_* parsed from graph_source in fabro-petri/src/fork_stage_envelope.rs; create lints in check.rs; checkpoint write-guard in hooks.rs + staged_paths in checkpoint.rs; FsScope carried in fabro-pebble-sandbox/src/fs_scope.rs; worker wiring petri_worker.rs; pin tests/fork_stage_envelope.rs |
| W4 | fabro-71a8/afab/fdd8/8795/d0dd/d420 | Web re-ports; probe+guards; server ops (approval TTL, env compat, capability gate, staleness); wait endpoint; small CLI verifications; superseded proofs |
| W5 | fabro-d659 | Cutover runbook (era check, backup, deploy, supervised pass, denkhaus archive) |

## Local-run preconditions (learned W0, 2026-09-21; staging addendum W3-5)

- Staging = the local docker stack (`just up`). Runs target
  `denkhaus/fabro@denkhaus-petri` explicitly (the scheduler is off; runs
  are triggered manually via API or CLI). The image bakes the
  sandbox-driver plugins at the workspace-pinned rev (fabro-96c6:
  `cargo dev docker-build` builds sandbox-driver-docker/-host from the
  Cargo.toml pin; Dockerfile COPYs them). The dev image carries no plugin
  checksum pin: `.env` sets `PETRI_SANDBOX_PLUGIN_DEV=1` for the local
  stack; production pins `PETRI_SANDBOX_DOCKER_SHA256` instead.
- ALWAYS `env -u FABRO_SERVER` — the agent shell exports a local dev
  server and parse-tests assert `server.is_none()`.
- Docker tests need sandbox-driver plugins on PATH (workspace-pinned rev)
  and pre-pulled images: `ghcr.io/lithoscomputer/ubuntu-24.04:{slim,dind}-<RUNNER_PIN>`
  plus CATALOG_IMAGE `ghcr.io/lithoscomputer/ubuntu-22.04:slim`.
- Run workspace suites with `--profile ci` timeouts and `ulimit -n 8192`.
