# Revision — run 01M20T9S8JRXETASN64768TRRP

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M20T9S8JRXETASN64768TRRP.md
- seeds filed: fabro-fc1b — Implementer prompt: require the full touched-crate test suite when the seed touches tests
- basis: run 01M20T9S8JRXETASN64768TRRP, workflow version 8aedc5a9bbdd71b2f882af0b6c7d28a947212553646a31a390f5d173a91c8a69, commit 9966d125e665593fcd4f21137a19659625d58ed0
- revised_at_commit: 9966d125e665593fcd4f21137a19659625d58ed0 (ADR-0015: engine drift signal for later judgement)

## Findings

### Implementer prompt: require the full touched-crate test suite when the seed touches tests
- filed: fabro-fc1b
- change: in `.fabro/workflows/develop/prompts/implementer.md` step 4, replace the "ONE focused test" rule with a full touched-crate pre-gate (`cargo nextest -p <touched-crate>` when the seed adds or edits tests in a crate), never the workspace suite.
- expected effect: implementer@1's targeted-only testing missed two pre-existing Docker-socket failures in the full fabro-server suite; the gate-red bounce cost ~22 min (~40% of the 55-min run wall) and ~$0.8 to apply a 6-line `[run.environment] id = "local"` pin. Full-crate pre-gating surfaces such breaks in the same implementer pass. No duplicate: closed fabro-0d56/fabro-2254 cover fmt/clippy scope, not test-suite scope.
