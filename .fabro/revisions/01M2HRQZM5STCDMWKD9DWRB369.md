# Revision — run 01M2HRQZM5STCDMWKD9DWRB369

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2HRQZM5STCDMWKD9DWRB369.md
- seeds filed: fabro-7bb3 — Revisor intake: require filed seeds to name a concrete target file path
- basis: run 01M2HRQZM5STCDMWKD9DWRB369, workflow version e9a1ee7a6c5ba11dd8043f9696a924221a033c9861a170e832e1d16e220cde50, commit 09b612750e6d4cbc39ced42bf17e55e735bad695
- revised_at_commit: 09b612750e6d4cbc39ced42bf17e55e735bad695 (ADR-0015: engine drift signal for later judgement)

## Findings

### Revisor intake: require filed seeds to name a concrete target file path
- filed: fabro-7bb3
- Concrete change: in `.fabro/workflows/revisor/prompts/file.md` procedure step 1, the bookkeeper refuses or repairs a finding whose description references its target only vaguely (e.g. "the analyst prompt" instead of `analyze.md`). This run's claimed seed `fabro-6c14` was vague; the develop planner flagged it (seq 42) and burned 3 discovery probes (seq 43, 49, 55) ≈15 s. Distinct from fabro-3839, fabro-7f27, fabro-aa46, and closed fabro-cf76.
- Expected effect: planner passes skip the target-discovery detour whenever they pick a revisor-filed seed.
