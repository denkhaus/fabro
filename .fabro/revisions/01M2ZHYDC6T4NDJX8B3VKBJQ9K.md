# Revision — run 01M2ZHYDC6T4NDJX8B3VKBJQ9K

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2ZHYDC6T4NDJX8B3VKBJQ9K.md
- seeds filed: none — zero filing credit this pass (no same-pass stale/superseded closes; ADR-0022)
- balance: 0 non-exempt seeds filed / 0 — no credit this pass
- basis: run 01M2ZHYDC6T4NDJX8B3VKBJQ9K, workflow version 8d3a780e5a0665da9cb87b36306f1ef7e10841bbc767996f563f828c08a277b6, commit 6df980dc19a526baf1c000c97cd6d9ee7faeb57d
- revised_at_commit: 6df980dc19a526baf1c000c97cd6d9ee7faeb57d (ADR-0015: engine drift signal for later judgement)

## Findings

### Default planner-preflight candidate depth from 5 to 10

- filed id: none — overflow-to-journal (zero credit this pass)
- concrete change: in `.fabro/workflows/develop/scripts/planner-preflight.nu` raise the `--top: int = 5` default (line 227) to 10. In this run the preflight table covered only the top 5 `sd ready` seeds while the actionable seed fabro-9114 sat below them, forcing the planner to hand-adjudicate landed-ness over two extra LLM rounds and two shell calls (~$0.056, ~30s of a 97.1s planner pass). Cost scales ~2.3s/candidate (11.7s for 5 → ~23s for 10), still far under the 120s node timeout; full payoff requires fabro-679a's direct-commit landed-set fix, which this complements.
- dedupe: open fabro-c3b4 (top-N sd ready view in planner) is thematic overlap on a different mechanism — cross-referenced, not superseded. Next pass may consolidate with the other open `planner-preflight.nu` overflows (externally_gated arm, suffix-resolve, out-of-repo target, stranded salvage) as ONE multi-arm seed when balance allows.

- overflow: Default planner-preflight candidate depth from 5 to 10 — raise `--top: int = 5` default (line 227) to 10 in `.fabro/workflows/develop/scripts/planner-preflight.nu`; effect: actionable seeds below the top-5 `sd ready` window surface mechanically instead of costing ~2 extra planner LLM rounds (~$0.056, ~30s per affected run), ~2.3s/candidate cost, well under the 120s node timeout

### Skip PR creation for bookkeeping-only run diffs

- filed id: none — overflow-to-journal (zero credit this pass)
- concrete change: engine-side PR-open path (or a per-graph `run.pull_request` condition): when the final run diff touches only `.fabro/journal/**` and `.seeds/**`, push the branch and let the next real merge sweep it instead of opening a PR. In this run the entire diff was journal plus one seed-row edit (2 files, +5/−1), yet PR #327 was opened with a full CI gate and auto-merge on `denkhaus` — steady PR/CI churn per no-cycle run across 116 develop runs. Coordinate with open fabro-9495, which keeps the loop-asset PR but skips the project gate: this finding owns the PR-open policy decision.
- dedupe: open fabro-9495 (skip project gate, keep PR) and open fabro-b1d3 (Slack annotation for bookkeeping runs) cover adjacent mechanisms — cross-referenced, neither superseded; no existing seed names PR-open skipping.

- overflow: Skip PR creation for bookkeeping-only run diffs — engine PR-open path (or per-graph `run.pull_request` condition) skips PR creation when the final diff touches only `.fabro/journal/**` and `.seeds/**`, pushing the branch for the next real merge to sweep; effect: eliminates full CI-gate + auto-merge churn per no-cycle run (PR #327 shape, 2 files +5/−1, across 116 develop runs), coordinated with open fabro-9495

### Exempt engine response-dedup keys from the context_allow_keys drop notice

- filed id: none — overflow-dup of an already-open ledger entry
- concrete change: the planner node's `context_allow_keys` in `.fabro/workflows/develop/workflow.fabro` (or the fabro-900e lint itself) should accept the engine-generated response-dedup output key (`output.planner`, the fabro-b907 mechanism) instead of emitting `context_allow_keys dropped` at warn level on every planner pass — observed as run.notice at seq 90 of this run. Expected effect: one fewer spurious warn per run, keeping the envelope-lint signal clean for genuine contract drift.
- overflow-dup: add `output.planner` to the planner node's `context_allow_keys` in `.fabro/workflows/develop/workflow.fabro` (open in .fabro/revisions/01M2WCNY3WGP4QFZ24KPACBTND.md) — same theme, adjacent to the open authoring-lint overflow in .fabro/revisions/01M2YC82ECXM5KY8EC1TPNPRDK.md
