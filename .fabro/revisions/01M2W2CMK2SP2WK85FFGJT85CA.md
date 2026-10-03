# Revision — run 01M2W2CMK2SP2WK85FFGJT85CA

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2W2CMK2SP2WK85FFGJT85CA.md
- seeds filed: none — two findings survived dedupe but pass had no credit (ADR-0022)
- balance: 0 non-exempt seeds filed / 0 — no credit this pass (no same-pass stale/superseded closes)
- basis: run 01M2W2CMK2SP2WK85FFGJT85CA, workflow version 341257a3c3ccf500ad9774c1dab92b6aef751603b31e66b83b7c48131841f51b, commit 470b795d2369b43ae74e52d0f44b37db672fd17c
- revised_at_commit: 470b795d2369b43ae74e52d0f44b37db672fd17c (ADR-0015: engine drift signal for later judgement)

## Findings

### overflow: raise reviewer preamble_output_max_lines 200 → ~400
- Target: reviewer node in `.fabro/workflows/develop/workflow.fabro`
- Concrete change: `preamble_output_max_lines` 200 → ~400. In this run the ~9,361-byte (~300 diff line) evidence capture rendered as "(113 lines omitted)"; the omitted head held the integrity header and the rotate-review.nu seed-work diff, forcing 3 recovery git shell calls (events seq 328–360). The 16 KB `preamble_inline_max_kb` byte cap is not binding at this size — the line cap is. Open fabro-cf3e covers only the KB axis (16→32), open fabro-9837 only the blob-marker axis; no existing seed covers the reviewer preamble line cap.
- Expected effect: reviewer sees full evidence captures without re-reading the evidence commit tree; avoids a potential Verification-blocked re-capture cycle.

### overflow: planner preflight blocked_upstream verdict for upstream-PR-blocked seeds
- Target: `.fabro/workflows/develop/scripts/planner-preflight.nu`
- Concrete change: add a verdict arm that reads a seed body's "UPSTREAM PR OPEN / close when #N merges" status block and emits `verdict: "blocked_upstream"`, excluded from candidates. In this run the planner burned 5 tool calls (~50 s, ~12 K tokens, incl. a 13.4 KB web_fetch of fabro-sh/fabro#784) adjudicating fabro-af22 — top of `sd ready` but unactionable — and the planner journal notes this recurs every run (events seq 43–71). Open fabro-d9f7 (stale requeue) and fabro-0195 (automations config) do not touch the ready-list exclusion mechanism.
- Expected effect: removes ~50 s / ~12 K tokens recurring planner cost per run; stops re-adjudicating the same top-of-list seed.
