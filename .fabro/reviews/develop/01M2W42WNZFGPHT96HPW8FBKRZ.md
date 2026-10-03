# Improve review — run 01M2W42WNZFGPHT96HPW8FBKRZ

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (6.7 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-19 06:12+0000 by revisor `fabro_ask`

---

Run facts used below (from run events and the run conclusion): 9/9 stages green on the first pass — no gate reds, no review cycles. Wall 384.6 s, cost $0.512 (planner $0.142 / implementer $0.315 / reviewer $0.056). Implementer: 239.3 s inference vs 5.4 s tool time. Planner: 64.1 s inference, 7 tool calls. Seed claimed and closed: **fabro-7611** (crate-relative anchor resolution in `anchor_check.nu`), PR #268.

## Recommendations, ordered by expected impact

**1. Ship the vendored nushell skill — the implementer's 3 broken drafts were pure nu-semantics re-derivation.**
Seed: **fabro-d19e** ("Add a nushell-scripts skill so the implementer stops re-deriving nu 0.115 semantics").
Evidence: the implementer stage was 61% of run cost and 97.8% inference (239.3 s of 245.3 s wall, 25 messages, 1 errored shell call). Its own journal + lesson mx-3e5766 record three pitfalls hit blind (empty `str replace` replacement throwing inside try/catch, `[$x | cmd y]` list-literal pipeline absorption, no `glob --directory`). The Rust-guide hard gate didn't apply — the brief explicitly said "nushell script change; no Rust files are touched" — so no guide was loaded at all.
Change: create `.fabro/skills/nushell-scripts/SKILL.md` (seed mx-3e5766 content as the starter page) and add a nu-script arm to the implementer prompt's "read the vendored guide FIRST" gate in `.fabro/workflows/develop/prompts/implementer.md` (and the reviewer's standards axis, step 3).
Effect: nu-script seeds stop burning 2–3 debug drafts; on this run's shape that's roughly 60–90 s and ~$0.10–0.15 per run.

**2. Teach the preflight to classify externally-blocked seeds from their body status.**
New-seed justification: no existing seed covers body-status-driven skip classification — fabro-0da8 (tracker guard node), fabro-d9f7 (stale in_progress requeue) and fabro-ead4 (duplicate misclassification) are different mechanisms.
Evidence: the planner spent ~39 s of its 75 s stage (events seq 45–62: `sd show fabro-af22`, two git-log/rg probe rounds, a `git fetch`, three reasoning turns) re-deriving what the seed body's first line already stated ("UPSTREAM PR OPEN: fabro-sh/fabro#784"). ~52% of planner wall, ~$0.07.
Change: in `.fabro/workflows/develop/scripts/planner-preflight.nu`, detect explicit status markers in seed bodies (`UPSTREAM PR OPEN`, `blocked on upstream`) and emit a verdict `externally_blocked` in the `output.preflight` candidate table, so the planner skips on sight.
Effect: planner laps skip blocked-on-upstream candidates mechanically; on this run it would have halved the planner stage.

**3. Replace the 200-seed firehose with a top-N view and batch the recon calls.**
Seed: **fabro-c3b4** ("Use a top-N sd ready view in the planner instead of the full firehose"); complementary: fabro-55a7 (batch reconnaissance into one shell call).
Evidence: the planner's first shell call returned **28,749 bytes / "200 ready issue(s)"** with `stdout_truncated: true` (event seq 38) — only the top 5 were ever used; the preflight had already adjudicated those 5. Six further one-at-a-time shell calls followed.
Change: planner prompt (`prompts/planner.md` step 1) + preflight: pass a top-N limit to `sd ready` (the preflight's candidate set size, ~5–10) and chain `sd show` + base greps into single labeled calls per fabro-55a7.
Effect: ~25 KB less planner context per pass (planner conversation already hit 39.9 k tokens, 4.9% of the window) and fewer round trips — the cheapest planner-cost cut available.

**4. Copy the `rg -rn` footgun warning into the planner prompt — it fired live this run.**
Seed: **fabro-6997** ("planner prompt: two footgun one-liners — fs_hide glob returns empty, never rg -rn").
Evidence: event seq 57, the planner literally ran `rg -rn "find_skill_references" lib` — the exact documented footgun (from run 01M22X87J1RKQ, where `-r n` was parsed as replace-with-literal-n). The useful matches in that output came from the separate `rg -ln` in the same command; the follow-up call had to re-probe. The warning currently exists only in `implementer.md` ("rg flag discipline").
Change: add the one-liner to `.fabro/workflows/develop/prompts/planner.md` (per fabro-6997, which also covers the fs_hide-glob-returns-empty line).
Effect: eliminates a silent garbage-output probe class from the role that shells most before claiming.

**5. Derive `files_touched` from the stage git diff.**
Seed: **fabro-f8f8** ("Derive files_touched from the stage git diff, not write-tool calls").
Evidence: the implementer edited `.fabro/workflows/develop/scripts/anchor_check.nu` entirely through shell (heredoc/sed — required, the path is fs_hide-bound for file tools), so its stage `files_touched: []` and `last_file_touched: null` despite a +62/−9 diff landing in the checkpoint. Write-tool-based tracking is structurally blind on every loop-asset seed.
Change: engine-side — populate `files_touched` from the per-stage checkpoint diff (the diff the run already computes).
Effect: stage telemetry matches reality for all fs_hide-scoped seeds; downstream consumers (fidelity, reports) stop under-reporting.

**6. Capture per-criterion check outputs in the evidence pipe for implementer runs too.**
Seed: **fabro-d89a** ("Evidence capture: emit per-criterion check outputs for implementer runs, not just verification-only runs").
Evidence: the reviewer re-ran all three fixture verdict rows live via shell (3 calls, 0.245 s tool) because the evidence capture can't show runtime behavior — exactly the duplicated verification fabro-d89a targets. It was cheap here, but it also drove reviewer inference (28.8 s) re-adjudicating claims the capture couldn't pin.
Change: `.fabro/workflows/develop/scripts/evidence.nu` — when the implementer's summary names check commands, run them and embed their outputs as a per-criterion section in the capture.
Effect: reviewers judge from the capture; tool round trips and re-verification reasoning shrink on every implemented seed.

**7. Bound the `fabro_runs_list` in-flight call.**
Seed: **fabro-6b58** ("Bound the planner in-flight check: created_since window plus explicit self-exclusion").
Evidence: the planner called `fabro_runs_list {workflow: "develop"}` with no `created_since` (event seq 43) — 9.1 s round trip returning 76 runs, which it then had to eyeball for open PRs/non-terminal statuses.
Change: planner prompt step 4 + preflight: always pass `created_since` (e.g. 48 h) and exclude self, per the seed.
Effect: the mandated in-flight check drops from a 76-row scan to a handful of rows, seconds of planner wall every run.

**8. Scope per-node memory and the sd-command table by role.**
Seed: **fabro-9588** ("Per-node skills and memory scoping for agent stages"); complementary: fabro-52b4 (split PROJECT_FACTS sd table per role).
Evidence: all three agent sessions loaded the identical 25,746-byte `AGENTS.md` (events seq 29, 96, and reviewer init) — planner (tracker-only role), implementer, and reviewer alike; the reviewer also received the full sd-command table for commands it is forbidden to run, and the planner (which never touches Rust) received the pinned-toolchain block. Cache-read-heavy but repeated 3× per run (992 k cache-read tokens total).
Change: engine per-node memory scoping per fabro-9588; prompt-side, render only the role's slice of PROJECT_FACTS per fabro-52b4.
Effect: smaller preambles across all three roles; on a 1 M window it's headroom, but it compounds with rec 3 on every cycle and re-plan.

**9. Extend member-root resolution to `check-bare-paths` (the fabro-60a0 residual).**
New-seed justification: fabro-7611 (closed by this run) was explicitly scoped to `check-anchor` line anchors; no open seed covers the same wrong-base class in `check-bare-paths`, and the implementer's journal flags it report-don't-fix.
Evidence: this run's own preflight still shows `fabro-60a0` flagged `missing_file` for `prompts/survey.md` — the implementer confirmed the bare-path checker resolves against repo root + develop workflow dir only, not workspace member roots.
Change: `.fabro/workflows/develop/scripts/anchor_check.nu` — route `check-bare-paths` through the new `resolve-anchor-path`/`member-src-roots` stage landed this run.
Effect: the last false `missing_file` class disappears from preflight verdicts; planners stop adjudicating phantom flags.

**Minor footnote:** event seq 79 shows `run.notice context_update_dropped: output.planner` — benign (fabro-0a4c's fix merged `context_updates` correctly; only the redundant `output.planner` envelope drops), but it fires on every planner pass. New-seed justification: fabro-0a4c is closed and no seed tracks this residual notice; either declare `output.planner` in a consumer's `context_allow_keys` or demote the notice to debug.

What I could not fully verify: absence of duplicate seeds for the two new-seed items was checked against this run's 200-entry `sd ready` output and the tracker rows visible in run diffs — the `sd ready` listing was truncated mid-list, so a filing-time duplicate check (per fabro-aa46) is still advisable.
