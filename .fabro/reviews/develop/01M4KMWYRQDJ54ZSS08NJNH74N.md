# Improve review — run 01M4KMWYRQDJ54ZSS08NJNH74N

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: failed (5m31s, revisor pass — reason and cost in run detail)
- generated: 2026-10-10 23:57+0000 by revisor `fabro_ask`

---

All evidence is in. Here's the analysis.

## What actually happened in this run (from run events + workspace)

Start 19:33:46Z → terminal 19:39:17Z (5m31s wall, $0.363, 89% of wall was inference). Guards were cheap and correct: `env_guard` 44ms, `tracker_guard` 288ms, `preflight` 3.4s, `claim_check` 117ms. `planner@1` (73s, $0.106) claimed **fabro-c543**; `implementer@1` ran 226s ($0.257, 71% of run cost), 15 tool calls, 3 test files rewritten, diff checkpointed (+182/−280) — then its **final model request died on `llm:rate_limit`** (zai 429, code 1308, 5h window, reset 05:04:33). The `Blocked` edge routed it to `planner@2`, which hit the same 429 in **0.7s**, then `Planner failed` → exit (soft_stop/transient_infra). Six seconds after exit, the engine **created PR #391** with auto_merge+squash enabled. The run had `model.fallbacks = {}` (no failover).

## Recommendations, ordered by expected impact

**1. Configure a model fallback chain — the quota wall must not be a single point of death.**
- Evidence: this run died at 87% implementer completion with `fallbacks: {}` (run spec); the same 429 class killed the 2026-09-30 pass (fabro-ce9c fresh occurrence) and the 2026-09-11 outage day (fabro-91bf). Per `lab/fabro/failures.md`, quota errors are *never* retried against the same provider but *are* failover-eligible — the engine had a legal escape and no chain configured.
- Change: `.fabro/workflows/develop/workflow.toml` → add `[run.model.fallbacks]` under the existing `[run.model]` (e.g. `"glm-5.3" = ["<second provider>"]`). The per-node allowlists are unaffected.
- Expected effect: a zai 5-hour window degrades the run to a fallback model instead of terminating it; this run would have reached tester/reviewer instead of parking at the summary message.
- Seed: **fabro-e7e5** (open) — its first arm is literally "Configure fallbacks in the run's model settings" (its basis run also had `fallbacks = {}`); this run supplies the quota-side basis.

**2. Stop publishing auto-mergeable PRs for failed runs.**
- Evidence: PR #391 was created at 19:39:23, six seconds after the failed exit, for work that never reached `tester`/`reviewer` — and the work is **incomplete**: from the run checkout, `fork_git_identity.rs` still defines its own `host_plugin_ready` (line 55) and `no_questions` (line 270); only 2 of the 3 fork files in fabro-c543's criterion (1) were deduped. With `auto_merge = true` + squash, a green Dogfood Gate on the branch can merge half-done, never-reviewed work into `denkhaus`. Worse for the loop: the open PR makes every later preflight mark fabro-c543 `in_flight`, parking the seed behind PR #391's fate.
- Change: engine publish path (pipeline/pull_request) or `[run.pull_request]` in workflow.toml — create PRs only on success-terminated runs, or force `draft = true` and skip the auto-merge enable on non-success terminals; the pushed run branch remains the salvage instrument (the fabro-b4ed salvage-pointer pattern).
- Expected effect: unreviewed partial work can no longer auto-merge; a soft-stopped run's seed isn't parked behind a PR the loop itself can never merge.
- New-seed justification: no existing seed covers PR-creation gating on failed runs (fabro-2e66 is degradation *surfacing*; fabro-895d/b4ed handle stranded/retired PRs, not creation).

**3. Route `llm:rate_limit` on the implementer straight to the soft exit — not into a doomed planner re-entry.**
- Evidence: `implementer@1` failed 19:39:15.071 with `failure_class: "llm:rate_limit"`; the `Blocked` edge (condition `outcome=failed`) immediately spawned `planner@2`, which hit the identical 429 in 0.7s — a guaranteed-dead call against a 5-hour window. The same implementer→planner→exit sequence is documented in fabro-ce9c's fresh occurrence.
- Change: `workflow.fabro`, before the `Blocked` edge: `implementer -> exit [x.kind="soft", label="Quota window (next run re-enters)", condition="outcome=failed && failure_class=\"llm:rate_limit\""]`. `failure_class` is engine-written to context and explicitly routable (lab/fabro/failures.md); model-chosen Blocked verdicts don't carry this class, so genuine re-plans are unaffected.
- Expected effect: no futile LLM call; the terminal event attributes the park to the implementer's quota failure with the reset prose intact (what the quota-park classifier keys on), instead of arriving via a second failed stage.
- New-seed justification: fabro-ce9c records the identical sequence as a classification *observation only* ("no new decision implied"); no seed covers the graph routing change.

**4. Actually register the in-flight exclusion tool — it's been silently off, and this run proves it.**
- Evidence: planner journal (run events): "fabro_runs_list tool unavailable in this planner environment; in-flight check relied on preflight table — journaled degraded mode." Two causes, both verified: `prompts/planner.md:32` names the pre-rename tool `fabro_runs_list` (graph declares `fabro_run_search`), and **workflow.toml has no `[run.agent]` section** — this run's spec shows `agent.fabro_tools: false`, and per fabro-43cf's e2e evidence the run-wide flag is required for registration, so even the correct name wouldn't load.
- Change: implement **fabro-9b84**'s rename in `.fabro/workflows/develop/prompts/planner.md`, and add `[run.agent] fabro_tools = true` to `.fabro/workflows/develop/workflow.toml` (the per-node `x.fabro_tools="fabro_run_search"` already names and bounds the tool; implementer/reviewer stay denied via their empty lists).
- Expected effect: the planner's step-4 guard genuinely enumerates open-PR/non-terminal runs (what the branch-scan can't see) instead of degrading every pass; narrows the claim race (fabro-a01f).
- Seed: **fabro-9b84** (open) covers the prompt rename; the `[run.agent]` enable is the uncovered config half — extend 9b84's implementation with it (9b84 alone will not restore the tool).

**5. Fix the two preflight anchor false-positive classes this run paid for.**
- Evidence: 2 of 5 ready candidates flagged. `fork_run_tool_pins.rs` (bare filename, actually at `lib/components/fabro-workflow/tests/`) and `fork_tool_backend_double.rs` (the seed's *proposed* target file) flagged `missing_file` for fabro-1d5a; fabro-c543 got a `mismatch` flag on a citation artifact — the planner adjudicated all of it by hand (journal observations 1–2) during its 73s lap, and the next planner will re-derive the same fabro-1d5a resolutions because the flags regenerate every preflight.
- Change: `.fabro/workflows/develop/scripts/planner-preflight.nu` (anchor arm) — resolve bare filenames by rglob before emitting `missing_file`, and apply the add/create-naming window that `claim-check.nu` already has to proposed target files.
- Expected effect: false anchor flags disappear for bare-filename and proposed-file citations; planner laps stop re-deriving path bases by hand (fabro-1d5a becomes claimable without manual adjudication).
- New-seed justification: fabro-7611 (closed) fixed only crate-relative anchors against workspace member roots; bare-filename rglob resolution and the proposed-file window are uncovered (fabro-e8ae is a different extraction bug).

**6. Make the terminal notification an actionable park message.**
- Evidence: the run posted `run.failed` to `#dev-fabro` (workflow.toml `[run.notifications.terminal]`) with no next step, while the failure message carries the exact reset deadline ("will reset at 2026-10-11 05:04:33"). This run was user-fired via CLI, so ADR-0021 rev2's gate re-fire + rewind recovery doesn't apply — a human must find the resume path by reading stage events (exactly fabro-e566's original complaint).
- Change: terminal notification payload for quota-class soft stops — include the reset time and the concrete recovery command (`fabro rewind`/resume for this run) instead of the bare failure text.
- Expected effect: the operator gets a copy-paste recovery instead of event forensics; parked manual runs get revived at window reset instead of being re-derived by a fresh run.
- New-seed justification: the e566/a3d8 lineage covered park *classification*; no seed covers the actionable resume hint in terminal notifications.

**Not inspectable from here:** PR #391's live merge/auto-merge state and check results (no GitHub access from this analyst), the content of the implementer's 1 errored shell call (event search returned no match for the tool-error event), and whether any later run has since rewind-resumed this park.
