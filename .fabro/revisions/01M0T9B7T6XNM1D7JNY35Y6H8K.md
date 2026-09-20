# Revision — run 01M0T9B7T6XNM1D7JNY35Y6H8K

- status reviewed: failed
- review: .fabro/reviews/develop/01M0T9B7T6XNM1D7JNY35Y6H8K.md
- seeds filed: fabro-b065 — Blank stale seed context keys when routing Tracker empty; fabro-d936 — Make seeds-cli exit non-zero on errors

## Findings

### Blank stale seed context keys when routing Tracker empty
- filed: fabro-b065
- Change: extend the `Tracker empty` JSON contract in `.fabro/workflows/develop/prompts/planner.md` to also blank `current_seed_id`/`title`/`brief` and `implementation_summary`. Expected effect: the closing planner pass stops hauling stale seed context as blob refs — fewer blob round-trips, smaller preambles, lower evidence-starvation risk.

### Make seeds-cli exit non-zero on errors (exit 0 masks failed tracker writes)
- filed: fabro-d936
- Change: `seeds-cli` should exit non-zero and print errors to stderr instead of stdout with exit 0. Expected effect: removes a silent-corruption path for tracker writes and eliminates exit-code-checking gates reading failure as success. Complements prompt-side fabro-1399.
