# Pre-Petri inventory — `origin/denkhaus-0` (tip `85440fac6`, 2026-09-21)

Read-only survey. Evidence = `git show origin/denkhaus-0:<path>` (+ line where useful).
`HEAD` = branch `denkhaus` (today). No checkout, no build, no tracked-file change.

## Headline numbers

| Metric | `denkhaus-0` | today (`denkhaus`) | note |
|---|---|---|---|
| Tracked files | 3940 | 3798 | 3501 common, 439 old-only, 297 today-only |
| `lib/components` crates | 23 | 20 | 6 crates exist only at old |
| `lib/foundation` crates | 21 | 20 | `fabro-core` gone |
| `lib/apps` crates | 4 | 4 | same names |
| `fabro-workflow` src files | 131 | 17 | heavy shrinkage |
| `apps/fabro-web` files | 337 | 343 | near-identical |
| `docs/public` files | 282 | 282 | identical count |
| `.fabro/workflows/*` | 23 | 25 | old-only `code-review`; today-only `loop`,`cutover-probe`,`ask-e2e` |
| `.fabro/journal/*.jsonl` | 313 | 349 | persisted journal stream |
| `.fabro/scripts/*.nu` | 9 | 25 | today much richer |

Crates that exist **only** at `denkhaus-0` (files today = 0): `fabro-hooks` (8),
`fabro-sandbox` (31), `fabro-validate` (54), `fabro-acp` (8), `fabro-core` (14), `fabro-mcp` (6).

## Axis 1 — Run hooks: present

- `lib/components/fabro-hooks/src/lib.rs`: `pub use runner::HookRunner; pub use types::{HookContext, HookDecision, HookEvent, HookExecutionContext};`
- Definition layer `lib/foundation/fabro-config/src/layers/run.rs:763`:
  `#[serde(deny_unknown_fields)] pub struct HookEntry { id, name, event, matcher, blocking, timeout: Option<Duration>, sandbox: Option<bool>, script: Option<InterpString>, command: Option<Vec<InterpString>>, url, headers, tls, prompt, model, max_tool_rounds, agent }`
- Resolution `lib/foundation/fabro-config/src/resolve/run.rs:635`: `fn resolve_hook(hook: &HookEntry, ...) -> HookDefinition` — `script` is folded into `HookDefinition.command`; 4 transports (`command`/`http`/`prompt`/`agent`).
- Events `lib/foundation/fabro-types/src/settings/run.rs:2503`: `pub enum HookEvent { RunStart, RunComplete, RunFailed, StageStart, StageComplete, StageFailed, StageRetrying, EdgeSelected, ParallelStart, ParallelComplete, SandboxReady, SandboxCleanup, CheckpointSaved, PreToolUse, PostToolUse, PostToolUseFailure }` (+ `is_blocking_by_default`).
- Execution `lib/components/fabro-hooks/src/executor.rs:177`: sandbox hook gets `FABRO_HOOK_CONTEXT` (JSON temp file), host hook gets the same JSON on **stdin** via `sh -c` with `cmd.current_dir(wd)`.
- **Host-side repo-file access**: `src/types.rs:140` `fn command_cwd_for(&self, definition)` → host hooks run in `host_source_dir`, sandbox hooks in `sandbox_work_dir`. So a `sandbox = false` hook is cwd'd at the host checkout **locally / bare-server**. Caveat, quoted from `.seeds/issues.jsonl` fabro-8e13: "in production the fabro server runs CONTAINERIZED … NO host checkout mounted, so a sandbox=false hook executes INSIDE the server container" (hence `nu` must be vendored into the image).
- Config in use: `.fabro/workflows/develop/workflow.toml:9` `[[run.hooks]] name = "stage-journal" event = "stage_complete" script = "nu .fabro/scripts/stage-journal.nu" blocking = false timeout = "10s" sandbox = true`; carried by 5 of 23 workflows (architect, conductor, develop, merge-upstream, revisor).

## Axis 2 — FS policy / stage envelopes: present

- `lib/components/fabro-sandbox/src/fs_scope.rs:1-3`: "Per-stage filesystem scope (fabro-ba96, ADR-0009 stage envelope)". `pub struct FsScope { hidden: Vec<WorkspaceGlob>, write_allow: Option<Vec<WorkspaceGlob>> }`; `check_read` / `check_write` / `is_path_hidden`; denials `ScopeDenial::{HiddenByFsHide, OutsideFsWrite}`.
- Semantics: `fs_hide` = path behaves as if absent (reads fail, writes/deletes denied, listings filtered, `dir/**` hides `dir` itself); `fs_write` = allow-list, empty list = read-only stage; reads outside the workspace pass through.
- Enforcement seam = the agent tool layer (file/search/list/`apply_patch`), inherited by subagent sessions. Trust model is explicit: "drift protection, not adversarial containment (ADR-0009): `shell` and process execution remain the documented escape hatch" — and journals record the bypass as a real daily friction (`.fabro/journal/*.jsonl`: "`read_file` … is blocked by fs_hide … had to fall back to shell grep/sed").
- Attribute spelling at `denkhaus-0` is plain: `.fabro/workflows/develop/workflow.fabro:170` `fs_hide=".fabro/**,.seeds/**,.mulch/**,.agents/**,scripts/**,justfile"`.

## Axis 3 — Context tokens vs preamble budget: partial (interpolation ABSENT, preamble present)

- `{{ context.NAME }}` **absent** — it is an open seed at `denkhaus-0` (`.seeds/issues.jsonl` fabro-e71b): "Stage prompts cannot interpolate run CONTEXT values today: TemplateContext (lib/foundation/fabro-template/src/lib.rs) binds only goal + inputs.* + vars.*, all rendered during materialization". (That seed is **closed** at `HEAD` → today has it.)
- Preamble present and rich: `lib/components/fabro-workflow/src/handler/llm/preamble.rs:62` `pub fn build_preamble(fidelity, context, graph, completed_nodes, node_outcomes, output_max_lines) -> String`, fidelity modes `Truncate|Compact|SummaryLow|SummaryMedium|SummaryHigh|Full`.
- Budget knobs: graph/node attrs `preamble_budget_kb` (aggregate; engine default 24, `.fabro/workflows/develop/workflow.fabro:27` `preamble_budget_kb=48`), `preamble_inline_max_kb`, `preamble_output_max_lines` (`DEFAULT_PREAMBLE_OUTPUT_MAX_LINES`), and the read-side allow-list `preamble_allow_keys` (docs/public/execution/context.mdx:314 "restricts which context keys render in the node's `## Context` section").
- Value delivery to agents is the `## Context` section plus the `context_read` tool (ADR-0009) — not inline tokens.
- Validator owns it: `lib/components/fabro-validate/src/rules/{preamble_allow_keys_exist,preamble_inline_max_within_budget,preamble_stages_ignore_targets_exist}.rs` (crate absent today).

## Axis 4 — Journal / per-stage observability: present

- `.fabro/scripts/stage-journal.nu:5`: "SHARED by all three loop workflows (conductor, develop, revisor): each workflow.toml references this ONE copy from its `[[run.hooks]]` block". Schema `fabro-journal-v1`, one line per stage execution, file `.fabro/journal/<run_id>.jsonl`; fields `run_id,node,visit,status,ts,data`.
- Engine side: `HookContext.context_updates` bridges the stage's declared `journal` payload (seed fabro-31b2), fed by `lib/components/fabro-workflow/src/lifecycle/hook.rs:24` `pub hook_runner: Option<Arc<HookRunner>>`.
- Persisted evidence at `denkhaus-0`: 313 `.fabro/journal/*.jsonl` files, e.g. `.fabro/journal/01M30CR038H873SSWDZKYJJSBY.jsonl:3` `{"$schema":"fabro-journal-v1","run_id":…,"node":"analyze","visit":1,"status":"succeeded",…,"data":{…}}`.
- Known defects at that tip (still open at `HEAD`): fabro-850f empty `data:{}` records, fabro-d308 first-invocation exit 1.

## Axis 5 — Exit kinds / conditional edge kinds: present (different syntax)

- `lib/components/fabro-workflow/src/lifecycle/mod.rs:440-450`: on an edge to an `Msquare` exit node, `edge_kind = ctx.edge…str_kind_attr("kind")`, then `edge_kind.or_else(|| target.str_kind_attr("kind")).unwrap_or("natural")`, stored at `context::keys::INTERNAL_EXIT_KIND`. Precedence: edge `kind` > exit-node `kind` > `natural`.
- Classification `lib/components/fabro-workflow/src/pipeline/finalize.rs:208`: `Some("deadlock") => FailureReason::Deadlock`, `Some("soft") => FailureReason::SoftStop`; success path adds `(Some("boundary"), _, _) => SuccessReason::Boundary` (fabro-08b4), plus `PublishBlocked` / `PartialSuccess`.
- Today's Petri-native spelling `a -> b [x.kind="soft"]` does **not** exist at `denkhaus-0`.

## Axis 6 — Quota park + provider readiness gates: present

- `lib/apps/fabro-server/src/server/fork_line_recovery.rs:1` "Fork-only line recovery: provider window gate (fabro-986b, user decision 2026-09-14 … 'vor jedem cron run muss der llm provider auf 429 gecheckt werden')"; `pub(crate) const RECHECK_INTERVAL_SECS: i64 = 600`; "BEFORE every scheduled fire … the scheduler probes the provider … Window closed -> NO run is created".
- `pub(crate) fn is_quota_park(status, failure)` uses the shared classifier `fabro_types::is_quota_rate_limit_failure` (SoftStop + TransientInfra + `rate_limit` signature), statuses `Failed{SoftStop}` / `Blocked{QuotaRateLimit}`; gate **fails open** without a resolvable model.
- Readiness: `lib/components/fabro-llm/src/client.rs:210` "A built client plus what the build learned about provider readiness"; `selection.rs` `catalog_fallback_recovers_from_readiness_failures_only`; availability probe seed 8d30a.
- Breaker/park plumbing: `lib/components/fabro-automation/src/breaker.rs`, `lib/apps/fabro-server/src/server/automation_scheduler.rs` (`park_run_with_signature`).

## Axis 7 — Sandbox providers: present

- `lib/foundation/fabro-types/src/sandbox_provider.rs`: `pub struct SandboxProviderKind(Cow<'static,str>)` ("Open by design: the bundled providers (`local`, `docker`, `daytona`) run in-process, and any other kind names a sandbox-driver plugin executable"); `pub enum BundledProvider { Local, Docker, Daytona }`; `enum WorkspacePolicy { DesignatedDirectory, Clone }`.
- `lib/components/fabro-sandbox/` (31 files): `docker.rs`, `daytona.rs`, `driver.rs`, `provider.rs`, `driver_sandbox.rs`, `provider_sandbox.rs`, `pebble_environment.rs`, `reclaim.rs`, `reconnect.rs`, `git_policy.rs`, `redact.rs`, plus `tests/plugin_provider.rs` and `tests/daytona_streaming_live.rs`.

## Axis 8 — Run persistence / resume: present

- Operations `lib/components/fabro-workflow/src/operations/`: `archive, create, fork, fork_resume_from_failure, lifecycle_events, resume, retry, rewind, run_store, source, start, timeline, validate` (14 files). `rewind` is the primitive (`fork_resume_from_failure.rs:16` `use super::rewind::{RewindInput, RewindOutcome, rewind}`; `fork.rs:108` "Local folder runs execute in place without Git checkpoints; cannot fork or rewind").
- Store `lib/components/fabro-store/src/`: `record/{codec,record_id,repository}.rs`, `slate/run_store.rs`, `keys.rs`, `run_state.rs`, `run_summary_store.rs`, `legacy_run_history_import.rs`, `legacy_blob_import.rs`.
- Checkpoint identity `lib/components/fabro-checkpoint/src/{author.rs,trailer.rs}`; CLI `lib/apps/fabro-cli/src/commands/run/{checkpoints,resume,rewind,fork}.rs` (no `retry.rs` CLI at old; today adds it).

## Axis 9 — Web app + API: present, near-identical

- `apps/fabro-web/`: 337 files at `denkhaus-0` vs 343 today (today-only: `app/lib/petri-stream.ts`, `app/components/platform-records-panel.tsx`, `app/test-fixtures/petri/*.json`).
- `lib/foundation/fabro-api/`: 65 files at old vs 66 today; `lib/packages/fabro-api-client/` exists in both (594 files old vs 621 today). `docs/public/api-reference/fabro-api.yaml` present in both.
- So the SPA + API surface survives unchanged; the delta is Petri stream/record views, not the app's existence.

## Axis 10 — CLI command surface: present, ~same

- Top-level `lib/apps/fabro-cli/src/commands/` — old: `artifact auth automations config doctor dump env exec graph install mcp model parent parse pr preflight provider render_graph repo run runs sandbox secret server system uninstall upgrade validate variable version workflow`. Today: same set **minus `parse.rs`**, **plus `seeds/`**.
- `commands/run/` old: adds nothing we lack — it has `checkpoints, resume, rewind, fork, steer, ask, attach, cp, diff, events, logs, preview, ssh, wait, …` Today adds `retry.rs`, `timeline.rs`, `petri.rs`, `petri_stream.rs`, `petri_worker.rs`.

## Today's `denkhaus` capabilities with NO counterpart at `denkhaus-0` (sample)

| Path | Purpose |
|---|---|
| `lib/components/fabro-petri/src/engine.rs` (+ whole crate, 68 files) | Petri `apply(state,event)->commands` engine adapters; the new execution core |
| `lib/components/fabro-petri/src/admission.rs` | Workflow admission into Petri; run's display graph read off the admitted graph |
| `lib/components/fabro-petri/src/projection/**` | Record→view fold (`stream_seq` run stream, `fork_exit_kinds.rs`, `edge_conditions.rs`) |
| `lib/components/fabro-petri/src/hooks.rs` (1575 lines) | Lifecycle effect service **plus** user `[[run.hooks]]` execution (`HookDefinition`, `runs_in_sandbox`) merged from the old `fabro-hooks` crate |
| `lib/components/fabro-petri/src/fork_stage_envelope.rs` | Re-reads `x.fs_write`/`x.fs_hide`/`x.preamble_*` from raw `graph_source` (Petri drops `x.` attrs at lowering) + create-time lint (successor of `fabro-validate`) |
| `lib/components/fabro-dot/**` | Workflow graph shape + file references through Petri's DOT parser |
| `lib/foundation/fabro-db/migrations/2026091701_petri_records.sql`, `…1801_petri_projection.sql`, `…1803_drop_run_events.sql` | New `petri_records`/`platform_records` tables; `run_events` dropped |
| `lib/apps/fabro-server/src/sandbox_access.rs` | Server reaches a run's sandbox (sandbox tab, Run Files, terminal, preview, `fabro cp`, Ask Fabro) via the Petri record |
| `lib/apps/fabro-server/src/petri_runs.rs`, `server/{run_publish.rs,run_records.rs,stream_follower.rs}` | Petri run API, publish gate, record/stream endpoints |
| `lib/apps/fabro-cli/src/commands/run/{petri_stream,petri_worker,retry,timeline}.rs`, `commands/seeds/**` | New CLI surface for the Petri worker/stream, retry, timeline, seeds backend |
| `lib/foundation/fabro-types/src/{run_stream,session_event,engine,run_graph,notice,diagnostic,reference,agent_props}.rs` | New wire/domain types for the record projection feed |
| `apps/fabro-web/app/lib/petri-stream.ts`, `app/components/platform-records-panel.tsx`, `app/test-fixtures/petri/*` | SPA consumption of the Petri run stream |
| `.fabro/workflows/loop/**` (9 files), `cutover-probe/**` (6), `ask-e2e/**` (3) | New loop workflow, cutover probe workflow, ask E2E workflow |
| `lib/components/fabro-llm/src/{judgment.rs,judgment_replay.rs}`, `examples/judgment_replay.rs` | Judgment-shadow implementation (seed fabro-8e13, typesafe/jev) |
| `.fabro/scripts/` 25 files vs 9 | `battery-runner.nu`, `claim-check*.nu`, `salvage-sweep*.nu`, `pin-toolchain.nu`, `toolchain-guard.sh`, `judgment-shadow*.nu`, … |
| `lib/components/fabro-store/src/platform_records.rs`, `run_session_event_store.rs`, `run_summary.rs` | New record/event stores (old had `record/`, `slate/`, `run_state.rs`) |
| `lib/apps/fabro-server/src/server/{seeds_source.rs,handler/seeds.rs}`, `fork_seeds_git_source.rs` | Seeds tracker read/git-source API |
| `.fabro/workflows/develop/scripts/{check-transcript,claim-check-smoke}.nu` | New develop-leg deterministic checks |
| `lib/components/fabro-workflow/src/run_tools.rs`, `src/prompts/pr_body.md` | Run-tool registry + PR-body prompt |

## Would going back be feasible?

- The tree is ~89% identical (3501 of 3940 old paths are common), and the pre-Petri line's own migration analysis, `docs/lab/petri-integration-analysis.md` (present at both tips; 120 lines at old, 277 at `HEAD`), already scores every fork feature for re-import: it records that upstream deleted `fabro-core`, `fabro-sandbox`, `fabro-hooks`, `fabro-validate`, `fabro-acp`, and shrank `fabro-workflow` 131 → 17 src files.
- Returning to `denkhaus-0` restores all ten axes above, plus capabilities today lacks: the whole `code-review` workflow (74 files), `fabro-validate`'s rule family (54 files), `fabro-sandbox` (31), `fabro-hooks` (8), `fabro-acp` (8), `fabro-core` (14), `fabro-mcp` (6), and `{{ context.NAME }}`-free prompt/`preamble_budget_kb` semantics in their original (un-namespaced) attribute spelling.
- What a return loses is the Petri migration itself, still in flight at `HEAD` (epic fabro-9930 open): records/projection run stream, server-side `sandbox_access`, new CLI worker/stream/retry/timeline, the `loop`/`cutover-probe`/`ask-e2e` workflows, judgment-shadow, `x.`-namespaced graph attributes, and `run_events` (dropped irreversibly by migration 2026091803).
