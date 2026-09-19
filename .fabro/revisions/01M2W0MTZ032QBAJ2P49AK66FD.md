# Revision — run 01M2W0MTZ032QBAJ2P49AK66FD

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2W0MTZ032QBAJ2P49AK66FD.md
- seeds filed: none — filing balance: 0 credit this pass (no same-pass stale/superseded closes); sole surviving finding journaled as overflow
- balance: 0 non-exempt seeds filed / 0 — no credit this pass
- basis: run 01M2W0MTZ032QBAJ2P49AK66FD, workflow version 341257a3c3ccf500ad9774c1dab92b6aef751603b31e66b83b7c48131841f51b, commit d8da4c50a1160df9356930ee482cb9a20d52c57f
- revised_at_commit: d8da4c50a1160df9356930ee482cb9a20d52c57f (ADR-0015: engine drift signal for later judgement)

## Findings

### Reviewer node: raise `preamble_output_max_lines` 200 → ≥400, or emit an explicit blob-ref instead of silent line omission

- disposition: overflow-to-journal (ADR-0022: no credit this pass; not dropped)
- concrete change: in `.fabro/workflows/develop/workflow.fabro`, reviewer node — the evidence stage-section line cap (`preamble_output_max_lines=200`) silently omitted 190 of ~230 lines of a 14,206-byte capture that already fit the 16 KB inline budget, forcing the reviewer to burn 3 shell calls reconstructing the diff via `git diff 0fa14680` (checkpoint seq 272). Raise the line cap to ≥400 or route over-line-budget captures through an explicit blob-ref marker.
- expected effect: reviewer judges the full capture inline, no reconstruction calls, capture contract holds.
- dedupe note: analyzer pre-deduplicated against open `fabro-cf3e` (raises only the KB knob `preamble_inline_max_kb` 16→32), closed `fabro-meta-c9f2` (summary:high `tail_lines` renderer survival), and open `fabro-9837` (blob-ref marker metadata) — none covers the per-node line cap or the omission→blob-ref swap. Next pass must re-run its own dedupe before re-filing against its own balance.
