# Feature touchpoints — PETRI ERA (branch denkhaus)

The fork line lives on `denkhaus` (the petri line, renamed from
`denkhaus-petri` on 2026-09-27; base: upstream/main 40419cbd2, epic
fabro-9930 owns the port waves; full analysis:
`docs/lab/petri-integration-analysis.md`). The pre-petri world is archived
as branch `denkhaus-0`; its old-engine touchpoint rows live in that
branch's history and in the analysis doc — do not resurrect them here.

Merges walk `upstream/main -> denkhaus`. Every durable fork feature
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

| Duplicate-child guard (fabro-a875, W2) | fabro-tool/src/fork_duplicate_child_guard.rs (create.rs seam) | fabro-tool/src/fork_duplicate_child_guard_tests.rs | `cargo nextest run -p fabro-tool -E 'test(duplicate_child)'` |
| Publish protection, squash-revert + out-of-scope (fabro-2889/4ebd, W2) | pull_request.rs diff_touched_paths + out_of_scope_deletions; supervisor re-classify via fork taxonomy | fabro-workflow/tests/fork_publish_gate.rs | `cargo nextest run -p fabro-workflow --test fork_publish_gate` |
| PR retry + PR model (fabro-b5a9, W2) | fabro-github retry loop (survives); model resolution in operations/create.rs | inline fabro-github suite — pin-form gap, see audit | `cargo nextest run -p fabro-github` |
| Catalog overlay (fabro-6945, W3-pre) | fabro-llm/src/fork_catalog.rs (both codec-era builder paths) | inline tests in the fork-only file + fork-catalog-overlay fixture | `cargo nextest run -p fabro-llm` |
| Quota park on tiers (fabro-2e7b, W3) | fabro-petri park semantics; server/fork_line_recovery.rs is_quota_park | fabro-petri/tests/fork_quota_park.rs | `cargo nextest run -p fabro-petri --test fork_quota_park` |
| Exit kinds deadlock/soft + the one DOT edge scan (fabro-288d W3; scan hardened by fabro-8615) | fabro-petri/src/projection/fork_exit_kinds.rs and fork_dot_edges.rs (the shared comment-/quote-aware scan; edge_conditions.rs reads the same scan); coordinator.rs conclusion seam | inline mod tests in all three fork-only modules incl. the real revisor graph, plus the run-level pins in fabro-petri/tests/projection.rs (`a_plain_exit_beside_a_soft_kind_edge_projects_green`, `a_sole_soft_kind_edge_still_downgrades_a_green_finish`) | `cargo nextest run -p fabro-petri -E 'test(fork_exit_kinds) | test(fork_dot_edges) | test(edge_conditions) | test(soft_kind_edge)'` |
| Tolerant structured-output decode (fabro-4c11) | fabro-llm/src/fork_structured.rs (complete_object_tolerant: schema attached as response format, prose/fence-tolerant reply decode on top of lithos Client::complete_object); consumed by fabro-workflow pull_request.rs PR content | inline tests in the fork-only file | `cargo nextest run -p fabro-llm` |
| Checkpoint exec guard (fabro-0c08) | fabro-petri/src/fork_exec_guard.rs (bounded cooldown-ladder retry for the OCI resource-unavailable exec class at checkpoint's run seam; upstream owns checkpoint.rs — this file is the fork-owned half) | fabro-petri/tests/fork_exec_guard.rs (presence pin exercising the public guard surface) | `cargo nextest run -p fabro-petri --test fork_exec_guard` |
| Conclusion guard (fabro-b00c, 2026-09-25) | fabro-petri/src/projection/fork_taxonomy.rs refuses_green_conclusion + coordinator.rs conclude wiring | fabro-petri/tests/fork_conclusion_guard.rs | `cargo nextest run -p fabro-petri --test fork_conclusion_guard` |
| Stage envelope (fabro-aa5f, W3) | fabro-petri/src/fork_stage_envelope.rs + fs_scope in fabro-redact (fork-only file, moved there when upstream deleted fabro-pebble-sandbox, 2026-09-25 merge) | fabro-petri/tests/fork_stage_envelope.rs | `cargo nextest run -p fabro-petri --test fork_stage_envelope` |
| Preamble family enforcement (fabro-70af PART 2b) | fabro-petri/src/fork_preamble_policy.rs (FabroPreamblePolicy onto petri's PreamblePolicyHandle seam) + runtime.rs capability install + fork_stage_envelope.rs family parsing; enforcement lives in denkhaus/petri fork commit 0456785 (branch fabro: attractor/steps fork_preamble_policy.rs, core/ir fork_kv.rs tombstones), Cargo.lock pins it | inline presence tests in fork_preamble_policy.rs (the_preamble_family_lowers_onto_the_seam_policy, the_capability_answers_with_the_envelope_policy) + envelope family tests | `cargo nextest run -p fabro-petri fork_preamble_policy fork_stage_envelope` |
| x.tools per-node allow-list (fabro-1a41, fabro-70af PART 2a; commit 2b283180b) | fabro-petri/src/tool_policy.rs (ToolPolicyHooks at the HookService session seam, wired in runtime.rs; x.fabro_tools pass-through, clone inheritance) | tool_policy.rs inline tests | `cargo nextest run -p fabro-petri tool_policy` |
| Availability probe + sandbox guards (fabro-afab, W4) | server probe on sandbox lifecycle | server/src/server/fork_inspection_guard_tests.rs | `cargo nextest run -p fabro-server -E 'test(fork_inspection)'` |
| Server ops: staleness supervisor (fabro-fdd8, W4) | server/src/server/fork_staleness_supervisor.rs | server/src/server/fork_staleness_supervisor_tests.rs | `cargo nextest run -p fabro-server -E 'test(staleness)'` |
| Web re-ports (fabro-71a8, W4) | apps/fabro-web on RunProjection/RunStream | TS tests in-tree (run-actions.test.ts, automations-*.test.tsx); no fork-only naming for web yet | `cd apps/fabro-web && bun test` |
| fabro_ask + docs analyst (fabro-43cf, W5-pre) | fabro-tool ask.rs + catalog + ClientBackend wire; workflow dispatch; session routes; handler/fork_ask_docs.rs corpus | fabro-tool/tests/fork_ask_tool.rs + fabro-workflow/tests/fork_ask_tool_registry.rs | `cargo nextest run -p fabro-tool --test fork_ask_tool; cargo nextest run -p fabro-workflow --test fork_ask_tool_registry` |
| url.insteadOf rewrites (fabro-f394) | fabro-manifest/src/fork_insteadof.rs (longest-prefix match at URL-resolution time; manifest round-trips unchanged) | fabro-manifest/tests/fork_insteadof_tests.rs | `cargo nextest run -p fabro-manifest --test fork_insteadof_tests` |
| FABRO_STAGE stage-env injection (fabro-6e7f) | fabro-petri/src/fork_stage_env.rs (engine-injected step identity, agent-spoof scrub) + seams in runtime.rs/providers.rs/tool_policy.rs | fabro-petri/tests/hooks.rs stage-env case (`stage_processes_carry_the_dispatched_stage`) | `cargo nextest run -p fabro-petri --test hooks` |
| Seeds read API (fabro-3488, ADR-0023 step 5, fork decision B) | server/seeds_source.rs (SeedsSource seam, Disabled default → 503 seeds_source_unconfigured); server/fork_seeds_git_source.rs (GitRepoCache git mirror, branch-switch invalidation); `[server.seeds.mirror]` settings (fabro-types/fabro-config ServerSeedsSettings); seeds routes wired in server.rs build_router; OpenAPI /api/v1/seeds{,/graph,/{id}} | server/src/server/fork_seeds_read_api_tests.rs | `cargo nextest run -p fabro-server -- fork_seeds` |
| Loop lane: meta workflow + lane flags + run-scope (fabro-70b5) | .fabro/workflows/loop/**; shared lane flags in develop scripts (tracker-guard `--assignee`, planner-preflight `--assignee`, evidence `--lane`, closeout `--lane`); scripts/qualitygate.nu `check-run-scope` + product-tier run-scope check; prompt-lint loop glob | loop graph-contract smoke + run-scope fixtures, both wired into qualitygate's loop-asset tier | `nu .fabro/workflows/loop/scripts/graph-contract-smoke.nu && nu .fabro/scripts/run-scope-fixtures.nu` |
| Run-tool files_from (fabro-4b29) | fabro-tool workflow_version.rs (params+resolve), common.rs backend trait read_run_sandbox_files, fabro_client.rs impl; workflow run_tools.rs dispatch; server run_files.rs read_run_sandbox_files + handler/runs.rs GET /runs/{id}/sandbox-files (RequireWorkerRunScoped); OpenAPI listRunSandboxFiles; fabro-client list_run_sandbox_files | fabro-workflow/tests/fork_run_tool_pins.rs (schema keeps files_from + mutual exclusion) + dispatch equivalence test in run_tools.rs tests | `cargo nextest run -p fabro-workflow --test fork_run_tool_pins; cargo nextest run -p fabro-workflow -- files_from` |

## fcb2 audit addendum (2026-09-22 evening)

W2-W4 rows promoted above; pin-form status per closed wave seed:

- Solid fork-only pins: a875, 2889/4ebd, 6945, 2e7b, 288d, aa5f, afab,
  fdd8, 43cf (files verified in-tree).
- Pin-form gaps to resolve before closing fcb2:
  - b5a9: retry survives in fabro-github with inline suite only; a fork
    fork-only pin file is missing (upstream merge could drop the loop).
  - 8795 (wait endpoint): no route found by quick grep — closure evidence
    must name what actually landed (existing stream endpoint vs ported
    long-poll); re-verify before trusting the W4 row.
  - 788b / fa0a: RESOLVED — no pin needed. The fork extensions were
    intentionally not ported; Petri-native equivalents are documented in
    docs/lab/petri-integration-analysis.md (788b: per-node `fidelity`
    attribute replaces preamble scoping; fa0a: workflow-owned counters
    via context_updates + edge conditions replace engine-injected
    seed_cycles). Post-cutover tuning is revisor work, not migration.
  - 8795 (wait endpoint): RESOLVED — capability landed tool-shaped:
    `fabro_run_wait` (fabro-tool/src/wait.rs, the `n` tool; fork-added
    file, inline tests) supersedes the HTTP long-poll; conductor legs use
    it exclusively. Closed with evidence.
  - b5a9 + 8795 pin residual: fabro-c8e3 tracks ONE fork-only pin file
    in fabro-tool/tests/ covering PR-retry presence and the wait
    tool's terminal/merged contract (both are fork-added files with
    inline tests only).
  - 71a8 (web): TS tests are in-tree standard files; no fork-only naming
    convention exists for web. Decide: accept TS-test pin form or file a
    follow-up (user call).
- Suites (`fork_seam`-family runs above) must be green once before
  closing fcb2 (deferred while the staging rebuild holds the CPUs).

## Pending ports (wave seeds own the detail)

| Wave | Seed | Feature |
|---|---|---|
| W5 | fabro-d659 | Cutover runbook (era check, backup, deploy, supervised pass, denkhaus archive) |
| W3 inventory family | fabro-1392 / fabro-9b1b / fabro-a044 | Validation rules / hooks family / workflow transforms (W0 inventory gaps; hooks verified lowering-clean, execution rides fabro-petri hook tests) |
| W4 small verifications | fabro-d0dd | ask-duplicate, attach-retry, CLI small features (verified against suites; touchpoint rows only if code lands) |

## Local-run preconditions (learned W0, 2026-09-21; staging addendum W3-5)

- Staging = the local docker stack (`just up`). Runs target
  `denkhaus/fabro@denkhaus` explicitly (the scheduler is off; runs
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
