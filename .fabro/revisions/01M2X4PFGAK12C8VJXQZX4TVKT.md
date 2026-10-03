# Revision — run 01M2X4PFGAK12C8VJXQZX4TVKT

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2X4PFGAK12C8VJXQZX4TVKT.md
- seeds filed: none — healthy run, one finding overflowed the filing balance
- balance: 0 non-exempt seeds filed / 0 — no credit this pass
- basis: run 01M2X4PFGAK12C8VJXQZX4TVKT, workflow version 651b1a6e2a981a032674237f71e9a360311e8ba2901832de7fa9b0b3046836b7, commit ae4a107c268568103f6063c12726efaeae2b81b0
- revised_at_commit: ae4a107c268568103f6063c12726efaeae2b81b0 (ADR-0015: engine drift signal for later judgement)

## Findings

### Declare the engine response-dedup write in the planner's context_allow_keys
- filed: none — overflow to journal (ADR-0022: zero same-pass credit)
- concrete change: add `output.planner` to the planner node's `context_allow_keys` in `.fabro/workflows/develop/workflow.fabro`, or exempt engine-internal dedup keys from the allow-list check in fabro-workflow
- expected effect: the per-run false `context_update_dropped` warn disappears and warn-level signal becomes actionable
- dedupe: no existing seed covers this key/node (fabro-169b closed, same class different key; fabro-7028 is the gatebounce key; fabro-8275 covers preamble-absence warn)
