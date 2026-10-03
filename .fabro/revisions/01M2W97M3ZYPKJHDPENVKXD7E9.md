# Revision — run 01M2W97M3ZYPKJHDPENVKXD7E9

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2W97M3ZYPKJHDPENVKXD7E9.md
- seeds filed: none — all 4 findings survive dedupe but pass has 0 filing credit (no stale/superseded closes); journaled as overflow for the next pass
- balance: 0 non-exempt seeds filed / 0 — no credit this pass (no same-pass stale-superseded closes)
- basis: run 01M2W97M3ZYPKJHDPENVKXD7E9, workflow version 8b7bbe9d58b678ad56c49416d28afbf7872db2deec867d142043b12649cefcc8, commit 37c23e8c56a0326e5460a1e07cf961e39f2e1b37
- revised_at_commit: 37c23e8c56a0326e5460a1e07cf961e39f2e1b37 (ADR-0015: engine drift signal for later judgement)

## Findings

### Gate externally-blocked seeds in planner-preflight before the LLM lap (priority 1)
- overflow-to-journal (ADR-0022: no credit this pass)
- Change: add an `externally_gated` arm to `nu .fabro/workflows/develop/scripts/planner-preflight.nu` (parse candidate seed bodies for an open-upstream-PR Status section, e.g. fabro-af22's "UPSTREAM PR OPEN: fabro-sh/fabro#784"; mark it in the verdict table) plus one line in `.fabro/workflows/develop/prompts/planner.md` step 3: a seed whose body names an open external PR is journaled-skip, never tree-probed.
- Expected effect: removes ~$0.15/~75s per run while fabro-af22 tops the ready list (run 01M2W97M3ZYPKJHDPENVKXD7E9 spent ~73s/$0.154, 26% of run cost, seq 50-104, re-deriving line 1 of af22's body via 9 shell probes and 9 LLM round trips); planner laps start at candidate #2.
- Dedupe: no open seed covers externally-gated (upstream-repo) seeds; closed fabro-06e0 and open fabro-9372 cover only this repo's in-flight runs/PRs.

### Raise the reviewer node preamble_output_max_lines cap from 200 to ~300 (priority 2)
- overflow-to-journal (ADR-0022: no credit this pass)
- Change: in `.fabro/workflows/develop/workflow.fabro`, reviewer node, raise `preamble_output_max_lines` 200 → ~300 as a second arm alongside open fabro-cf3e's `preamble_inline_max_kb` 16→40 raise (orthogonal knob: lines vs bytes).
- Expected effect: reviewer judges from the evidence capture directly (run 01M2W97M3ZYPKJHDPENVKXD7E9 rendered "(52 lines omitted)" on an 8.8 KB capture under the 16 KB inline cap; the 200-line cap set by closed fabro-meta-c9f2 is what bit — reviewer compensated with 4 read-tool calls; same omission on a seed whose loop-work diff is the review scope is the documented "Verification blocked" escalation path).

### Lint filed seeds for dead seed-to-seed premises (priority 2)
- overflow-to-journal (ADR-0022: no credit this pass)
- Change: extend open fabro-3839's filed-seed lint with a seed-to-seed premise arm: flag seed bodies that name another seed's artifact while that seed is still open (orthogonal arm to fabro-3839's run-id Basis resolvability and tool-name typo scope).
- Expected effect: the fabro-d9f7-class contradiction (d9f7 'extends fabro-0da8's tracker-guard node' while fabro-0da8 is still open; run 01M2W97M3ZYPKJHDPENVKXD7E9 burned 5 probes/27s, seq 105-128, discovering it, then only journaled it) is flagged by the lint/preflight table instead of re-derived per run.

### Correct the PROJECT_FACTS fs_hide claim for the reviewer envelope (priority 2)
- overflow-to-journal (ADR-0022: no credit this pass)
- Change: add one clause to the develop PROJECT_FACTS block (`.fabro/workflows/develop/prompts/` facts include or `reviewer.md`): "fs_hide binds planner/implementer; the reviewer node is deliberately unhidden."
- Expected effect: removes a latent false-blocking path (run 01M2W97M3ZYPKJHDPENVKXD7E9's reviewer journal records `read_file` on `scripts/validate-workflows.nu` and `.fabro/workflows/*` succeeding while PROJECT_FACTS claims those paths are file-tool-blocked — a future reviewer trusting the prose could route "Verification blocked" on evidence it can actually read).
- Dedupe: open fabro-a512 corrects fs_hide claims in `workflow.fabro` comments only; no open seed covers PROJECT_FACTS prose accuracy for the reviewer envelope.
