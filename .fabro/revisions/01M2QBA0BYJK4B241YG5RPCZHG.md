# Revision — run 01M2QBA0BYJK4B241YG5RPCZHG

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2QBA0BYJK4B241YG5RPCZHG.md
- seeds filed: fabro-c0a2 — Fix quote-unaware diff_header_b_path parsing in publish.rs; fabro-22fa — Closeout: sweep reviewer-journal 'noted but not blocking' findings into open seeds
- basis: run 01M2QBA0BYJK4B241YG5RPCZHG, workflow version f4ab9ff48b6e162ee81f42689c29e27253fba4fe3632992c2e164afa31eb5ca5, commit 774aa49f80b3293ee8f15394d4e235f4fe93ffa4
- revised_at_commit: 774aa49f80b3293ee8f15394d4e235f4fe93ffa4 (ADR-0015: engine drift signal for later judgement)

## Findings

### Fix quote-unaware diff_header_b_path parsing in publish.rs
- filed: fabro-c0a2
- `diff_header_b_path` in `lib/components/fabro-workflow/src/pipeline/publish.rs` uses `rsplit_once(" b/")`, which mis-splits paths containing a literal ` b/` segment (git C-quotes space-bearing paths in real diffs), so the fail-closed arm silently reverts to PR-creation on such paths. Fix: decode git's quoted-p path form before matching. Observed by the run's own reviewer journal as 'noted but not blocking'.

### Closeout: sweep reviewer-journal non-blocking findings into seeds
- filed: fabro-22fa
- Extend the closeout re-file mechanism in `.fabro/workflows/develop/scripts/closeout.nu` to file reviewer-journal 'noted but not blocking' observations as open seeds before the claimed seed closes. Cross-references fabro-7aac (implementation-summary trigger) and fabro-534e — same extension pattern, different trigger source; not supersession.
