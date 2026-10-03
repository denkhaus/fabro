# Prompt generality & cross-workflow drift audit

Scope: conductor (survey, develop-leg, revise-leg, merge-leg), develop (planner, implementer, reviewer, project-facts), revisor (select, analyze, file) prompts + the three `stage-journal.nu` copies. HEAD tree verified against every staleness claim (seeds via `sd show`, runs via `.fabro/journal/`, shas/PRs via `git log`, graph state via `workflow.fabro`). No prompts were modified.

Legend: paths abbreviated `c/`=conductor/prompts, `d/`=develop/prompts, `r/`=revisor/prompts.

## Axis 1 — Seed-id / run-id / PR / sha / date leakage

Verified base rates: ~34 literal seed ids, 19 run ids, 9 PR numbers, 2 commit shas, 10 date pins across 11 files. Almost all are incident provenance "(evidence: run X, seed Y)" that resolves against the tree today; only 3 are rotten and 2 are behavior-load-bearing.

### (a) Load-bearing (would change agent action)

- c/survey.md:7,9-13,67-68 — `merge leg DISABLED (2026-09-13, user decision)` + "NEVER route Merge needed". Date-pinned behavioral switch; verified still true (merge node commented out in conductor/workflow.fabro:45-47,87,95-96). Rot risk: re-enabling the graph without reverting this block silently strands the merge route. — keep, but anchor to graph state ("while the merge node is commented out") instead of the date; it already half-does this.
- d/planner.md:31 — "Seeds without a Basis line are legacy (pre-2026-09-05)". Date cutoff classifies a seed population; mild behavior (legacy handling). — abstract to "seeds without a `Basis:` line are legacy".

### (b) Example-only / provenance (resolve today, rot slowly)

- c/develop-leg.md:13,17,29 — `#832` contract tag (x2 headings), `2026-09-10` duplicate-child incident, `fabro-8ee1`, `fabro-571e`, `fabro-bde4`, `fabro-978d`, run `01M2E2805XB4` + `2026-09-13`, `2026-09-05` (line 46). All resolvable (PR #832 = run-intent salvage merge; seeds closed as implemented; run = PR #138 merge commit f497572b5).
- c/revise-leg.md:8,13,29,33,35 — `#832` x2, `fabro-978d`, `fabro-571e`, run `01M2E2805XB4` + `2026-09-13`.
- d/planner.md:15,19,29,31,32,42,43,45,46 — 8 run ids, PR #109/#47/#115, shas `ae95484`/`7ae575c`, seeds fabro-{6baf,37a6,d936,05d0,06e0,22e4,91ff,9372,d183,a0e3,02c4}, dates 2026-09-05/09-09. All verified: runs have journals, shas exist, seeds resolve (d936/9372/6b58 still open as claimed; 22e4↔PR #47 mtime story checks out).
- d/implementer.md:24,29,32,33,35,72 — 7 run ids + seeds fabro-{6b58,9372,6e7f,0d56,1dae}. All resolve; fabro-6e7f in_progress matches "dispatcher exists".
- d/project-facts.md:42,51-52 — run `01M0T9B7T6` (--format failure), `nightly-2026-04-14` toolchain pin. Pin is live config, correctly placed in the facts file.
- r/analyze.md:34,44 — `fabro-7461` + 2026-09-02, `fabro-91ff` + 2026-09-09. Both resolve (closed, implemented).
- r/file.md:27 — `fabro-a0e3` absorbed into `fabro-45bf` by run `01M2368YQ`. All three resolve and match titles.
- d/reviewer.md:25 — PR #53 gh-baking revert; matches fabro-05d0 body (commit f00fdb102).

### (c) Stale / rotten (verifiably outdated vs tree)

- r/select.md:12 — `HARD PRECONDITION — sandbox (fabro-8d30a)`: seed no longer resolves (`sd show fabro-8d30a` → "Issue not found"; survives only as a citation inside open seed fabro-3377's body). The rule itself is sound; the citation is dead. — drop the id or re-point to fabro-3377.
- d/planner.md:43 — `justfile:128` cited as the qualitygate recipe's body location. justfile:128 is now a clean-target comment; `nu scripts/qualitygate.nu` lives at justfile:141. Line anchor rotted; the rule (`just qualitygate` / `nu scripts/qualitygate.nu` both banned) is still accurate. — cite the recipe name, not the line.
- c/merge-leg.md:19-20 — "Create child runs with the git workflow source — the server resolves and registers the workflow versions; the sandbox filesystem never participates". Contradicts the current contract (see Axis 4): develop-leg:31 "The server no longer resolves git workflow sources for tool-created runs", and merge-leg's own line 21. Leftover pre-#832 sentence; fabro-e297's body confirms sandbox-side resolution was the failure mode, not the fix.

## Axis 2 — Domain-specific overreach

The develop workflow has the right seam: project-facts.md is the declared single home for repo values (ADR-0013, d/project-facts.md:3-8). Conductor and revisor have no analog, so their repo facts are inlined. Findings:

- c/develop-leg.md:16, c/revise-leg.md:24, c/survey.md:5 — `repo: "denkhaus/fabro"`, `branch: "denkhaus"`, upstream URL, `origin/denkhaus` hardcoded inline (3 files, 4+ sites). — verdict: abstractable-with-single-source. Acceptance power is not lost by a conductor-level facts block (same include mechanism develop uses); today the merge-target fact lives in 5 places (see Axis 3 #7).
- d/implementer.md:32 — full Rust policy inline: `cargo nextest run -p`, `cargo +<pin> fmt/clippy`, E0432 anecdotes, workspace-suite bans. The policy is already mechanized in `just verify implementer` (scripts/verify.nu, prompt line 32 admits "verify wins") and the pinned commands live in d/project-facts.md:49-52. — verdict: largely abstractable ("run the PROJECT_FACTS verify recipe; tester owns the gate; never widen scope"). Keep: the default-features clippy self-assessment rule and the full-gate ban (those govern judgment, not mechanics).
- d/implementer.md:32,35 — `fabro-server Docker-socket failures`, `fabro-server fixture drift` (product crate named in incident examples). — abstractable ("a pre-existing failure in a touched crate"); zero acceptance loss.
- d/implementer.md:35,72 — `inherit_parent_target`, `is_engine_stamped_key`, `pub fn n(key: &str...)` (product symbols as examples). — abstractable ("a pub fn consumed outside its own crate"); the rg discipline rule survives.
- d/project-facts.md:56-59, r/file.md:55, d/reviewer.md:25 — `resolve_workflow_env` in `lib/components/fabro-workflow/src/services.rs`, `lib/components/fabro-workflow/src/git_bridge.rs`. Source-file anchors verified current (services.rs:395). In project-facts: keep-specific-by-design. In revisor file.md: overreach-by-copy (revisor has no facts file, so engine internals were pasted into a prompt).
- c/merge-leg.md:15, d/planner.md:29,32 — "Dogfood Gate", branch-protection naming. — keep-specific-because-acceptance (they name this repo's real CI contract); low rot risk since they describe behavior, not locations.

## Axis 3 — Duplication across workflows

1. Journal contract block — 10 near-verbatim copies: c/survey.md:53-63, c/develop-leg.md:50-60, c/revise-leg.md:44-54, c/merge-leg.md:24-34, d/planner.md:57-69, d/implementer.md:105-116, d/reviewer.md:31-43, r/select.md:32-34, r/analyze.md:73-75, r/file.md:66-68. Duplicated rule: "Report through `context_updates.journal` on EVERY pass. Silence is a missing report, not an empty one. Always emit BOTH keys … The engine records it durably per stage". Copies already drift: reviewer says "no last-writer-wins relay" vs "no rewriting" elsewhere; select.md dropped the rationale sentences entirely (see Axis 4 #3). — hoist to one shared include (the `project-facts.md` include mechanism already proves includes work).
2. Two-step workflow-version registration (#832) — 4 copies: c/survey.md:20-37, c/develop-leg.md:13-16+29-36, c/revise-leg.md:8-21+35-40, c/merge-leg.md:7-9+17-21. Duplicated rule: "runs come from immutable registered workflow versions … Inline {workflow, workflow_source} payloads are REJECTED". — one canonical paragraph + per-leg deltas.
3. Output hygiene rule — 9 copies: c/survey.md:72, d/planner.md:53, d/implementer.md:142, r/select.md:22, r/analyze.md:69, r/file.md:61 (+ short forms c/develop-leg.md:67, c/revise-leg.md:61, c/merge-leg.md:41). Duplicated rule: "Never write a bare slash-word surrounded by spaces — later agent stages parse such tokens as skill references and crash on them." d/reviewer.md has NO copy despite emitting `review_feedback` that flows into planner context — the one place the crash-mode is downstream-critical. — add to reviewer, hoist the rest.
4. GITHUB_TOKEN credential paragraph — 3 copies across 2 workflows: d/project-facts.md:56-59 ≈ r/file.md:55 ≈ d/reviewer.md:25. Duplicated rule: "the engine injects `GITHUB_TOKEN` into every agent shell call (`resolve_workflow_env` in `lib/components/fabro-workflow/src/services.rs`) and runs a git credential bridge (`lib/components/fabro-workflow/src/git_bridge.rs`)". Revisor re-pasted it because it has no facts include — worst drift risk (engine-internal paths change with refactors). — revisor facts include or a shared prompt partial.
5. sd duplicate-check rule — 2 copies in revisor: r/analyze.md:39 ("title matches are not enough; content duplicates hide behind different titles") ≈ r/file.md:15 ("content duplicates hide behind different titles"). — one sd command table per workflow, referenced not restated (develop already does this: planner points at PROJECT_FACTS, d/planner.md:23-25).
6. fs_hide escape-hatch explanation — 2 copies inside develop: d/implementer.md:62-71 ≈ d/project-facts.md:13-20 ("FILE TOOLS (read_file, write_file, edit_file, glob discovery) fail … the shell is unaffected"). Intentional reinforcement, but implementer restates the whole facts bullet. — keep one line in implementer + pointer.
7. Merge-target branch fact — 5 sites: d/project-facts.md:24-28 (`origin/denkhaus`), d/implementer.md:29 (inline `origin/denkhaus`), c/develop-leg.md:16 (`branch: denkhaus`), c/revise-leg.md:24 (same), c/survey.md:5 (`origin/denkhaus`). A branch rename currently edits five prompts. — single facts source per workflow (conductor lacks one entirely).
8. Duplicate-run guard — 3 formulations: d/planner.md:32 (in-flight runs via `fabro_runs_list`), d/implementer.md:29 (merged-commit grep on the merge-target branch), r/analyze.md:48-50 (closed-on-base grep on `<base-branch>`). Same intent, different mechanisms, different branch sourcing. — cross-workflow shared guard doc or at least a shared branch constant.

## Axis 4 — Contradictions between workflows

1. c/merge-leg.md:19-20 vs c/develop-leg.md:31-32 (and merge-leg's own line 21): "the server resolves and registers the workflow versions" vs "The server no longer resolves git workflow sources for tool-created runs … Inline payloads are REJECTED". Merge-leg is internally inconsistent and contradicts the sibling legs. (Dormant leg — see survey disable — but re-enabling inherits the stale sentence.)
2. c/merge-leg.md:9 "parent/target inherit" vs c/develop-leg.md:16 / c/revise-leg.md:27-29 "target is ALWAYS EXPLICIT `denkhaus` (omitted targets inherit the parent's RUN BRANCH and strand the work)". Merge-leg's omission is exactly the documented stranding bug (run 01M2E2805XB4) the other two legs patched. If the merge leg is re-enabled unchanged, it re-ships the bug.
3. Journal mandate strength: r/select.md:32-34 emits "Always emit BOTH keys" but drops "Silence is a missing report, not an empty one" and the `none`-valid clause every other prompt carries. Same topic, weaker rule in one workflow.
4. d/planner.md:1 "you are the only role that writes to seeds" is develop-scoped but phrased globally; r/file.md (Bookkeeper) files/closes seeds in the revisor workflow. Not a live conflict (different runs), but the unscoped sentence misleads anyone reading planner.md as loop-wide policy.
5. Hygiene asymmetry (drift, not strict contradiction): 9 prompts mandate the backtick/bare-slash-word rule; d/reviewer.md — whose `review_feedback` is consumed verbatim by the planner — has none.

## Axis 5 — stage-journal.nu copies

All three copies are byte-identical (sha256 prefix `2728fa1936607b1c`; `cmp` clean for conductor/develop/revisor). Zero divergence today. Risk is purely structural: 104-line script × 3 must move in lockstep with no shared source or sync check; the file's own header (seed fabro-176b) is the only provenance. Suggestion: single shared asset or a CI/prompt-side sync assertion — not urgent.

## Summary

- Axis 1: ~74 literal ids/dates total; 3 rotten (select fabro-8d30a, planner justfile:128, merge-leg stale sentence), 2 load-bearing date pins (survey merge-disable, planner legacy cutoff), rest resolvable provenance.
- Axis 2: conductor+revisor lack the project-facts seam develop has; implementer step 4 re-states the mechanized verify policy; product crates/symbols appear only in examples.
- Axis 3: 8 duplicated rule families; worst: journal block ×10, registration ×4, credential paragraph ×3 across 2 workflows, merge-branch fact ×5.
- Axis 4: 2 hard contradictions inside dormant merge-leg; 3 soft mandate/phrasing drifts.
- Axis 5: scripts identical.
