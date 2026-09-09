# Revision — run 01M22YZBNJ0Y06H8C8C0G3VN3W

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M22YZBNJ0Y06H8C8C0G3VN3W.md
- seeds filed: fabro-652d — batch checkpoint pushes at terminal/soft-exit boundaries
- basis: run 01M22YZBNJ0Y06H8C8C0G3VN3W, workflow version 14ab451ff2f9375f4927ce7d994c63bc96f48f3c617f3baff15119472f3af3e4, commit 2168350e49d7e7c63a09dcd996ae773f7cc8a46f
- revised_at_commit: 2168350e49d7e7c63a09dcd996ae773f7cc8a46f (ADR-0015: engine drift signal for later judgement)

## Findings

### Batch checkpoint pushes at terminal/soft-exit boundaries instead of every stage boundary — filed fabro-652d
Keep per-stage commits but push only at terminal or soft-exit boundaries. Run evidence: 95.4 s active stage time vs 141.6 s wall; each of 5 stage boundaries cost ~5 s (snapshot 2.1 s + commit + push; implementer done 11:32:44.4 → tester start 11:32:49.1); terminal segment (final push + PR creation) 17 s. Orthogonal to fabro-c2ca (skip snapshots on non-agent stages) and fabro-cf03 (snapshot async/branch-point-only) — neither covers push cadence; cross-referenced in the seed. Expected effect: ~10–20 s (~10%) off every develop run's wall time.
