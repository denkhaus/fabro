# Petri-era closed-work classification (read-only inventory)

Date: 2026-10-09/10 session inventory. Repo /home/denkhaus/dev/fabro, line branch `denkhaus`.
Read-only: no tracker writes, no tracked-file writes (this file lives in gitignored tmp/).

## Method and sources

- Closed-seed source 1: `iterate-state.json` -> `notes` (52 entries, 2026-10-01 .. 2026-10-09). Every `fabro-xxxx` token in a note was resolved against `.seeds/issues.jsonl`; the 64 with status=closed are Table 1.
- Closed-seed source 2: the explicit Petri port wave. `docs/lab/petri-integration-analysis.md` (2026-09-20) names the port children W0-W5 under epic fabro-9930; those closures (2026-09-20..22) predate the sprint ledger and are absent from the notes. Table 2 lists them plus the other closed seeds carrying labels `petri-migration`/`cutover`.
- Pre-Petri evidence: branch `origin/denkhaus-0` (tip 2026-09-20 23:45, archive commit 85440fac6, 0 fabro-petri paths, is an ancestor of HEAD) plus ADR-0007/0009/0010/0011/0019/0021/0022/0026 and `.agents/skills/merge-upstream/references/touchpoints.md`.
- Classes: REBUILD = the capability already existed pre-Petri and is being re-established/ported (including a defect in a rebuilt seam); NEW = capability with no pre-Petri counterpart; FIX = repair/refactor/policy/hygiene of code or assets that carried over (no re-implementation, no new capability); PRE-ERA = closed before the 2026-09-21 archive, outside the era denominator; UNCLEAR = not decidable from the evidence in hand.
- Rule used for gates/batteries/pins: a new enforcement over an existing rule counts NEW (the enforced rule is pre-Petri, the machinery is not). This rule is stated because it moves several rows (de32, 8b38, eae5, 8c89, 9973, a9bd).
- Dates are the tracker `closedAt` day; three fields are stale or empty (31b2, 44ac, and the blank-date rows), so the date is a hint, not proof.

## Table 1 - closed seeds named in the iterate-state notes (64)

| seed | one-line | class | evidence |
|---|---|---|---|
| fabro-0611 | b869 step 3: provider-window gate derives its provider set for never-run workflows (sprint 22) | NEW | provider-window gate (same family as b869) |
| fabro-06da | release scripts still tag with the GitButler workspace commit (sprint 21) | NEW | line-tip/release-integrity chain (no line-tip pin on denkhaus-0) |
| fabro-0c08 | Sandbox OCI exec EAGAIN at run finalize kills 54min of work (sprint 11) | REBUILD | sandbox provider facet rebuilt (denkhaus-0 fabro-sandbox; petri now owns sandboxes) |
| fabro-0da8 | Add a deterministic tracker guard node before the planner (sprint 12 reference) | PRE-ERA | closedAt 2026-09-19 (before the 2026-09-21 archive); denkhaus-0 already ships develop/scripts/tracker-guard.nu + smoke |
| fabro-17df | Implementer lesson-capture: require an ml record for reusable patterns (sprint 22 area) | NEW | tracker/journal hygiene machinery (ml/mulch records) |
| fabro-19f9 | agent sandboxes lack the Fabro git commit identity (sprint 17) | REBUILD | denkhaus-0 has fabro-workflow/src/git_identity.rs + a CLI git_identity test |
| fabro-1b2a | fabro_run_search lacks sandbox_available/workflow_version_id - revisor preconditions unsatisfiable (sprint 11) | REBUILD | denkhaus-0 revisor prompts/select.md documents both fields on fabro_runs_list |
| fabro-2093 | web automation toggle drops on_overlap and never updates UI (sprint 4) | FIX | automations UI + scheduler carried over (denkhaus-0 has web automations routes) |
| fabro-2357 | develop + revisor get the env_guard toolchain-placement node (sprint 24) | NEW | denkhaus-0 has no toolchain-guard/env_guard (develop/scripts carries tracker-guard only) |
| fabro-2a3b | Tracker integrity: a commit subject can claim a closure its diff never performs (sprint 18) | NEW | claim/close-check machinery; pre-Petri tracker-guard checks provenance, not close claims |
| fabro-2e57 | run title: no no-strict-JSON fallback (sprint 9) | REBUILD | same title-generation path (catalog/LLM seam rebuilt) |
| fabro-31b2 | hooks: carry stage context_updates in HookContext (journal bridge) | REBUILD | hooks family port (upstream deleted fabro-hooks; matrix W3 fabro-9b1b); note calls it 'old-engine knowledge, never ported' |
| fabro-3488 | Server read API: seed list/show/graph from host checkout (sprint 1) | NEW | new server route family; no seeds route on the pre-Petri branch (only repo-local .seeds/) |
| fabro-3ab2 | loop workflow must declare its environment - manual fires hit the default image (sprint 24) | FIX | the same untransmitted-[environments] gap is documented in denkhaus-0 .fabro/project.toml |
| fabro-3c11 | auth: a valid login under another host spelling is invisible (sprint 15) | FIX | auth store/CLI carried over (denkhaus-0 server auth/cli_flow.rs); UX defect, no port |
| fabro-3fce | skill-reference expansion kills petri agent sessions (sprint 23) | REBUILD | pebble patch expands slash refs only for typed input; skills feature is pre-Petri |
| fabro-40ba | a checked-in loop asset carries an embedded NUL byte (sprint 21) | FIX | the fixture file exists on denkhaus-0; file fix + one new lint line |
| fabro-44ac | DEPLOY PENDING: build + ship 0.362.0-fork.4 | NEW | fork release + deploy-window chain (ADR-0025 naming; no pre-Petri line-tip pin) |
| fabro-49af | embedded build sha names the workspace commit (sprint 21) | NEW | release/line-tip parity chain |
| fabro-4c11 | PR-content model selector never converted to the client route form (sprint 11) | REBUILD | matrix W2-3 PR-model-plumbing (pre-Petri fabro-890b; touchpoints.md row) |
| fabro-4d35 | Host toolchain kills fabro-proc title constructors (sprint 22) | FIX | fabro-proc (10 files incl. c/capture_argv.c) predates the migration; host env defect |
| fabro-51ad | nodes.<id>.generation conditions never fire - develop cycle guards dead (sprint 23 intake) | REBUILD | fork engine feature (generation guards); merge fallout on the new engine |
| fabro-55f1 | Loop scripts print LOCAL time with a hard-coded Z suffix (sprint 1) | FIX | defect in carried-over scripts (friction-score.nu/stage-journal.nu exist on denkhaus-0) |
| fabro-5af4 | Split code_review.py (3,968 lines) along its 14 section banners (sprint 7) | FIX | denkhaus-0 ships code-review/scripts/code_review.py (carried asset, refactor) |
| fabro-5c45 | Expect-absent .codex/instructions.md read logs ERROR twice per session init (sprint 14) | UNCLEAR | fs-probe log hygiene in the rebuilt sandbox fs layer; the pre-Petri fabro-sandbox read path was deleted upstream |
| fabro-647f | web tests: run bun test with --parallel (sprint 6) | FIX | test infra of the carried-over web app |
| fabro-6538 | rust-style-guide exists as two byte-identical copies with no parity check (sprint 6) | NEW | two copies predate the migration (denkhaus-0 .agents+/.fabro skills); the parity gate is new |
| fabro-6ac5 | routed-away rate-limit stage ends green - quota park only sees RunStatus::Failed (sprint 16) | REBUILD | denkhaus-0 automation_scheduler.rs fork note fabro-986b: 'a rate_limit signature is a quota park'; ADR-0021; matrix W3-1 |
| fabro-6e7f | Deterministic verify dispatcher `just verify` decides scope from step + diff (sprint 2) | REBUILD | denkhaus-0 already ships scripts/verify.nu + `just verify stage`; its header cites fabro-6e7f |
| fabro-70af | x.* family silently dropped by petri rework - re-implement probe-first, refuse unknown x.* (sprint 5) | REBUILD | label regression; migration matrix: fork envelope attrs die -> x.* namespace |
| fabro-767b | run_workflow.nu PR poll window shorter than platform latency (sprint 3) | REBUILD | denkhaus-0 has scripts/run_workflow.nu (fork publish script ported to the petri line) |
| fabro-79ba | revisor run: every stage green but run status=failed (sprint 11) | REBUILD | publish-blocked/projection port (matrix W1-3 fabro-6655 Projection-Fold) |
| fabro-846b | Lint bare slash-tokens in prompts of skills=discover nodes (sprint 24) | REBUILD | skills surface is a pre-Petri fork feature (matrix attribute survival) |
| fabro-8615 | Exit-kind classifier mis-parses comments - green revisor runs read Failed{SoftStop} (sprint 10) | REBUILD | ADR-0010 exit kinds; label petri-migration; matrix W3-2 fabro-288d |
| fabro-89f8 | line-tip-sha crashed in every but-less sandbox (loop pass 2) | NEW | line-tip/release-integrity machinery |
| fabro-8b38 | battery-registration parity: unregistered smoke/fixture battery must RED (sprint 12) | NEW | qualitygate registry machinery (denkhaus-0 has qualitygate.nu, no registration parity) |
| fabro-8bf6 | Delete stale cartography + config backup (.chisel frozen 2026-07-27) (sprint 12) | FIX | pre-cutover dead assets removed; hygiene, no capability |
| fabro-8c89 | dogfood-gate loop tier misses .fabro/** - skills/journal-only PR runs zero steps (sprint 21) | NEW | gate exists pre-Petri (dogfood-gate.yml); .fabro coverage + tier split are new |
| fabro-9067 | petri skills slash test RED: stage prompts never expand slash refs (sprint 23) | REBUILD | skills/slash family; labels fork-integration, petri-migration |
| fabro-9973 | Lane gates must run the checked-in-workflows snapshot before publish (sprint 5) | NEW | gate step with no pre-Petri row (the snapshot asset is pre-Petri; pre-publish gate is new) |
| fabro-9c44 | One tolerant DOT scanner for the four hand-rolled graph_source readers (sprint 14) | FIX | consolidates readers written during the port; DOT reading itself is pre-Petri (port cleanup) |
| fabro-a1ed | pin-toolchain must read the line tip via but sha (sprint 21) | NEW | same chain (the pre-Petri justfile has the recipe, not the parity gate) |
| fabro-a9bd | push-gate: name blocking PRs, separate run-PRs from lane-PRs (sprint 18) | NEW | push-gate.nu exists pre-Petri; the run/lane split + reporting is the new piece |
| fabro-a9cc | verify.nu v2: read FABRO_STAGE env (sprint 4) | REBUILD | same asset re-established (denkhaus-0 verify.nu v1 takes the stage as an argument) |
| fabro-ab2c | server: wire the auto-merge trigger (lab-world leak-guard) (sprint 2 reference) | PRE-ERA | closedAt 2026-08-26; seed text is dated 2026-08-26 (lab/denkhaus world, not the Petri era) |
| fabro-b46e | pre-fire readiness: configured provider without a stored credential must refuse (sprint 19) | NEW | no pre-Petri row (the breaker/pre-fire gate itself is the a52f port; credential readiness is new) |
| fabro-b60d | large-output projection test diverges live-vs-rebuild under load (sprint 15) | FIX | flake/fixture hardening in a petri-era test |
| fabro-b869 | provider-window gate v2: provider-scoped state, manual refusal, --force (sprint 20) | NEW | no window-scoped refusal pre-Petri (ancestry: ADR-0021 quota park, port seed 2e7b) |
| fabro-c16a | EPIC: migrate the code-review lane's python engine to nu (sprint 25 area) | FIX | 6 py files on denkhaus-0 code-review/scripts; language-sprawl migration of carried assets |
| fabro-cadd | Adopt the nu-agent sprint model (ledger + arch gate) | NEW | process/ledger machinery; closed not-planned (the line implements no sprint mechanics) |
| fabro-cb5c | prompt-lint reds salvage-sweep-smoke's synthetic fixture ids (sprint 24 burst) | FIX | prompt-lint predates (denkhaus-0 .fabro/scripts/prompt-lint.nu) |
| fabro-d10e | Retire seeds-cost-bench.nu (stale one-shot bench) (sprint 27) | FIX | stale-asset retirement |
| fabro-d367 | judgment-shadow host hook exits 1 on every stage (loop pass 3) | UNCLEAR | judgment layer is pre-Petri (ADR-0022, denkhaus-0 .fabro/scripts/judgment.nu); the shadow hook itself is new |
| fabro-d5b1 | run-title generation refused: catalog entry declares no structured output (sprint 9) | REBUILD | denkhaus-0 has fabro-server/src/run_title_generation.rs (capability pre-Petri) |
| fabro-ddb4 | admission parity battery: persist -> load -> model_requirements (sprint 21) | NEW | battery/enforcement machinery for the ported admission; no pre-Petri counterpart |
| fabro-de32 | touchpoints two-pin parity battery (sprint 11) | NEW | two-pin rule is pre-Petri (denkhaus-0 touchpoints.md); the machine parity net is new |
| fabro-e5ac | code-review pin wall: 22 sha256 pairs as one hand-made line (sprint 7) | FIX | same carried asset (pin wall regenerated) |
| fabro-e71b | Engine: {{ context.NAME }} interpolation at stage dispatch (sprint 23) | REBUILD | context tokens are pre-Petri (matrix: Run-Context kv.*); ported seam |
| fabro-e901 | fork_stage_envelope block walk not comment-/quote-aware (sprint 13) | REBUILD | ADR-0009 stage envelope; same class as 8615 (label engine/architecture) |
| fabro-eae5 | one battery runner for the loop tester gate (17 registered batteries) (sprint 26/27) | NEW | registry/runner machinery; the unregistered-battery class is new |
| fabro-f312 | Failed-run salvage sweep: line consumer files salvage-pointer seeds (sprint 18) | NEW | no salvage consumer pre-Petri (only manual .fabro/reports/*-wip-salvage dumps) |
| fabro-f394 | fabro-manifest ignores git url.insteadOf rewrites (sprint 3) | FIX | denkhaus-0 has lib/components/fabro-manifest (crate carried over; not a port artifact) |
| fabro-f93b | fork/retry bypass the provider-readiness refusal (sprint 21) | NEW | successor arm of the new readiness gate (residual of b46e) |
| fabro-f97f | Policy: 633 committed .fabro run-artifact files (sprint 6) | FIX | policy closure (ADR-0026); in-repo journaling is pre-Petri (ADR-0026/lab) |

## Table 2 - closed port-wave seeds NOT in the notes (the explicit rebuild core, 37)

| seed | one-line | class | evidence |
|---|---|---|---|
| fabro-fef5 | Petri W0: baseline denkhaus-petri - upstream green + dev-loop assets + feature inventory | REBUILD | explicit port wave (migration matrix W0) |
| fabro-6c16 | Petri W1: automations CLI family (fabro auto) on the petri API + client regen | REBUILD | matrix W1-1 (pre-Petri feature, score 7) |
| fabro-a52f | Petri W1: pre-fire provider gate + breaker exemption on automation_scheduler | REBUILD | matrix W1-2 |
| fabro-6655 | Petri W1: publish-blocked taxonomy + boundary exit kind in the projection fold | REBUILD | matrix W1-3 |
| fabro-fcb2 | Petri W1: presence-pin system v2 on the petri world | REBUILD | matrix W1-4 'Pins je Port neu' |
| fabro-1d89 | CLI: orphaned agent-session doc comment corrupted `fabro create --help` (W1-1 leftover) | FIX | port leftover; doc/help repair |
| fabro-c8e3 | Presence pin for PR-create retry (fabro-b5a9 residual) | REBUILD | two-pin rule applied to a ported feature |
| fabro-a875 | Petri W2: duplicate-child guard on petri create | REBUILD | matrix W2-1 |
| fabro-2889 | Petri W2: diff-based publish protection (squash-revert guard) | REBUILD | matrix W2-2 |
| fabro-b5a9 | Petri W2: PR-create retry + PR-model plumbing on the petri PR path | REBUILD | matrix W2-3 |
| fabro-6945 | Petri W2: fork catalog overlay on the lithos-llm codecs world | REBUILD | matrix W2-4 |
| fabro-2c17 | Sandbox files/exec access races the terminal container stop (Docker 409 -> API 404) | REBUILD | sandbox facet rebuilt on petri (labels petri-migration,cutover) |
| fabro-1392 | Petri W3: validation rule family onto petri check | REBUILD | matrix W3 (W0 inventory gap) |
| fabro-9b1b | Petri W3: hooks family ([run.hooks]) onto the petri hook service | REBUILD | matrix W3 (W0 inventory gap) |
| fabro-a044 | Petri W3: workflow transforms (stylesheet/model_resolution/variable_expansion) | REBUILD | matrix W3 (W0 inventory gap) |
| fabro-788b | Petri W3: preamble budget/compaction on attractor | REBUILD | matrix W3-3 (score 4) |
| fabro-fa0a | Petri W3: seed_cycles on petri run-context | REBUILD | matrix W3-4 (score 4) |
| fabro-aa5f | Petri W3: stage-envelope features (ADR-0009 stage_policy + context_read) | REBUILD | matrix W3-6 (score 4) |
| fabro-288d | Petri W3: exit kinds deadlock/soft on failure tiers + goal gates | REBUILD | matrix W3-2 (score 3) |
| fabro-2e7b | Petri W3: quota park on attractor failure tiers + wait steps | REBUILD | matrix W3-1 (score 3) |
| fabro-96c6 | Petri W3: workflow asset rework develop/conductor/merge-upstream on attractor | REBUILD | matrix W3-5 |
| fabro-71a8 | Petri W4: web re-ports on petri views (projection/stream) | REBUILD | matrix W4-1 |
| fabro-afab | Petri W4: availability probe + sandbox lifecycle guards | REBUILD | matrix W4-2 |
| fabro-fdd8 | Petri W4: server ops features (approval-TTL, environment_compat, capability_gate, staleness supervisor) | REBUILD | matrix W4-3 |
| fabro-d0dd | Petri W4: small CLI ports (ask duplicate, attach replay, spa_refresh, fs scope) | REBUILD | matrix W4-5 |
| fabro-8795 | Petri W4: wait endpoint parity | REBUILD | matrix W4-4 |
| fabro-d420 | Petri W4: superseded verifications + closures | FIX | verification-only closure of superseded rows |
| fabro-d659 | Petri W5: cutover runbook (era-check, backup, deploy, supervised pass, archive) | NEW | one-time cutover machinery (no pre-Petri counterpart) |
| fabro-10df | Petri migration: handoff restqueue until the W5 cutover (2026-09-22) | FIX | bookkeeping/handoff |
| fabro-b5bf | HANDOFF: migration complete, cutover pending (2026-09-22 night) | FIX | bookkeeping/handoff |
| fabro-5362 | Port fabro_ask (ADR-0011 Ask-Fabro tool) to the petri run-tools registry | REBUILD | explicit port; ADR-0011 is pre-Petri |
| fabro-43cf | Petri: port fabro_ask stage tool + docs-scoped analyst | REBUILD | explicit port (same family) |
| fabro-11d9 | fabro-github installation tokens lack workflows permission | FIX | GitHub App token scope; not a port artifact |
| fabro-843d | mini line: planner agent session stalls to soft_stop | REBUILD | soft_stop is an ADR-0010 exit kind (label petri-migration) |
| fabro-df60 | petri: API-created git-target runs fail at the start checkpoint bundle fetch | REBUILD | run start/checkout path rebuilt on petri (label petri-migration) |
| fabro-0664 | cmd::events insta snapshots predate the petri run.branch_published record | FIX | test snapshot drift (label petri-migration) |
| fabro-0130 | fabro_run_search omits sandbox_available/workflow_version_id (duplicate of 1b2a) | REBUILD | run-tool contract parity for the pre-Petri revisor selector |

## Counts

| class | Table 1 (notes, 64) | Table 2 (port wave, 37) | combined (101) | share of era-closed (99) |
|---|---|---|---|---|
| REBUILD | 20 | 30 | 50 | 51% |
| NEW | 23 | 1 | 24 | 24% |
| FIX | 17 | 6 | 23 | 23% |
| UNCLEAR | 2 | 0 | 2 | 2% |
| PRE-ERA | 2 | 0 | 2 |  |

- Era denominator: 101 closed rows minus 2 PRE-ERA (0da8 2026-09-19, ab2c 2026-08-26) = 99.
- Headline: REBUILD 50 (51%), NEW 24 (24%), FIX 23 (23%), UNCLEAR 2.
- Re-implementation load: 73 of 99 (74%) touch pre-existing capability (REBUILD restores it, FIX repairs it).
- 30 of the 50 REBUILD rows are the explicit W0-W5 port wave (Table 2); the remaining 20 are note-era work on rebuilt seams: exit kinds (8615, e901, 51ad), stage envelope/x.* (70af, 846b), quota park (6ac5), run-tool parity (1b2a, 4c11), title generation (d5b1, 2e57), git identity (19f9), context interpolation (e71b), hooks journal bridge (31b2), verify dispatcher (6e7f, a9cc), run_workflow publish (767b, 79ba), sandbox exec (0c08), skills slash (3fce, 9067).
- NEW clusters: provider readiness/window gate (b46e, f93b, b869, 0611), release/line-tip integrity (a1ed, 06da, 49af, 89f8, 44ac), gate/registry machinery (de32, 8b38, eae5, ddb4, 6538, 8c89, 9973, a9bd), tracker/claim hygiene (2a3b, 17df), salvage sweep (f312), lane env guard (2357), seeds read API (3488), sprint-model machinery (cadd).

### UNCLEAR rows and the evidence that would decide

- fabro-5c45: decide by comparing denkhaus-0's fabro-sandbox read path for `file_exists`-first semantics with pebble's `read_memory_file`; `read_memory_file` does not exist on denkhaus-0, so the probe is new-path code.
- fabro-d367: decide by diffing denkhaus-0 `.fabro/scripts/judgment.nu` against the petri-era `judgment-shadow.nu` for a shadow/hook mode; the shadow hook has no denkhaus-0 file.

## Open port-class seeds (labels cutover/petri-migration, or port/rebuild wording)

Open-seed inventory: 371 open. Labels: `petri-migration` 12 open, `cutover` 9 open. Narrow match on port/rebuild/migration wording in the title: 30 rows below.

| seed | labels | one-line | port-class verdict |
|---|---|---|---|
| fabro-9930 | [] | Petri integration: port fork features onto the new engine (branch denkhaus-petri) | PORT-CLASS (epic; all W0-W5 children closed, container still open) |
| fabro-4b29 | cutover,tool-surface | files_from port: fabro_workflow_version_create reads the closure from the run sandbox | PORT-CLASS (explicit port; label cutover) |
| fabro-0639 | cutover | petri staged-set envelope: inventory remaining deny-all agent stages for shell-mutated loop assets | PORT-CLASS (stage envelope = ADR-0009) |
| fabro-c5a0 | cutover | petri: agent-stage context_updates never reach the run context (kv) - stdin_source stages read empty stdin | PORT-CLASS (context tokens / run-context seam) |
| fabro-ef73 | cutover,checkout | Run-scratch checkout is depth-1: deepen for runs that ask for history | PORT-CLASS (run checkout path) |
| fabro-da8d | cutover,api,archive | Archive 404s for migrated legacy runs (summaries without petri records) | PORT-CLASS (migration data compatibility) |
| fabro-1437 | petri-migration | petri develop child: green cycle publishes no PR - orphaned old-era run branches fake an in-flight run | PORT-CLASS (publish/PR pipeline) |
| fabro-294d | petri-migration | petri projector: 'Petri run does not replay' on catch-all-concluded conductor passes | PORT-CLASS (projection fold) |
| fabro-2b55 | petri-migration | run title generation fails on short catch-all passes (recurring WARN) | PORT-CLASS (title generation is pre-Petri; see d5b1) |
| fabro-6ff3 | engine,petri-migration | petri projector b00c rule: a gatebounce-RECOVERED leg failure still downgrades the run | PORT-CLASS (projection/failure tiers) |
| fabro-ce9c | engine,petri-migration | green-lie over conditional failure routes: succeeded run with zero work | PORT-CLASS (exit kinds / failure tiers) |
| fabro-501a | engine,workflows,petri-migration | Planner output-contract JSON truncated by the node output cap | PORT-CLASS (engine output cap on the ported planner) |
| fabro-4791 | petri-migration | Progressive checkpointing: persist in-stage work so mid-stage failures lose minutes | PORT-CLASS (checkpointing; petri has it natively, fork behaviour missing) |
| fabro-5a77 | sandbox,petri-migration | sandbox driver plugin transport instability: 246 plugin-transport-failed errors | PORT-CLASS (rebuilt sandbox driver) |
| fabro-3a67 | petri-migration | Full-suite runs on dev hosts: load timeouts + Docker container leaks | PORT-CLASS (rebuilt sandbox/run loop) |
| fabro-5453 | workflows,architecture | world merger: develop+revisor onto the merged world, fabro as product (one-time migration) | PORT-CLASS (one-time migration tail) |
| fabro-c4be | workflows,petri-migration | develop line: preflight misses seed ids in squash-commit BODIES + implementer-hidden work | MIXED (line tooling fix; labelled petri-migration) |
| fabro-6f44 | petri-migration | fabro_seed: the tracker as a structured run tool for agent stages | MIXED (new run tool; labelled petri-migration) |
| fabro-70b5 | handoff,architecture,workflows,petri-migration | HANDOFF: build the loop workflow (meta lane) | MIXED (loop lane is petri-era new; labelled petri-migration) |
| fabro-6509 | handoff | Petri-Migration: Handoff-Restqueue bis W5-Cutover (Stand 2026-09-22) | BOOKKEEPING (handoff) |
| fabro-d4e4 | handoff,cutover | HANDOFF: CUTOVER DAY - migration green, suite 4412/4412, runbook ready | BOOKKEEPING (handoff) |
| fabro-b6c5 | handoff,cutover | HANDOFF: cutover steps 1-3 green on prod; step 4 blocked | BOOKKEEPING (handoff) |
| fabro-96aa | cutover | HANDOFF: fabro-4b29 files_from committed - UNDEPLOYED; deploy + steps 5-7 remain | BOOKKEEPING (handoff) |
| fabro-9b84 | cutover,workflow-assets | develop planner.md still names dead tool fabro_runs_list (graph migrated, prompt not) | PORT-CLASS (run-tools port residual) |
| fabro-8f42 | ready-for-agent | Cutover: retire sd/ml from repo + toolchain after proof bar | FIX-CLASS (retire survivors of the old world) |
| fabro-0722 | [] | Resume re-materialization: rebuild sandbox from run data when container was reaped | INCIDENTAL (keyword 'rebuild' only; new capability) |
| fabro-21c0 | revision | Port the workaround-is-a-painpoint clause to the implementer prompt's journal section | INCIDENTAL (keyword 'port' only; prompt edit) |
| fabro-53cc | architecture,testing,nu | arch(s24): every nu battery re-implements its own harness - one lib/battery.nu | INCIDENTAL (keyword 're-implements' only; refactor) |
| fabro-d584 | arch,sprint6-gate,workflows | arch: retire incident-pin graveyard workflows (incl. cutover-probe) | INCIDENTAL (keyword 'cutover' only; dead-asset retirement) |
| fabro-ed4d | architecture,workflows | conductor schema probe re-implements the schema's if/then in nu | INCIDENTAL (keyword 're-implements' only; refactor) |

- Verdict counts: PORT-CLASS 17, MIXED 3, FIX-CLASS 1, BOOKKEEPING 4, INCIDENTAL 5.
- The open port-class surface is small: 17 explicit residuals plus 3 mixed rows, against 50 closed REBUILD rows. The migration epic fabro-9930 is itself still open while all its W0-W5 children are closed.

## Limits

- The notes cover 2026-10-01 onward only; the port wave of 2026-09-20..22 is invisible there, so Table 1 alone UNDERCOUNTS the rebuild share. Read the two tables together.
- Titles/descriptions are the sessions' own claims; a few closures are bookkeeping or verification-only (d420, f97f, d10e) and carry no capability claim.
- Class assignment for gate/battery rows follows the stated rule; a different rule (gate belongs to its harness) would move ~7 rows from NEW to FIX and not change the REBUILD share.
