# Revision — run 01M2Q7VVHNQPTTEY14AFPBHFBZ

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2Q7VVHNQPTTEY14AFPBHFBZ.md
- seeds filed:
  - fabro-6ae9 — Planner: add compile-probe cost-tier rule (grep-first, timeout_ms >= 600000, never cargo run cold)
  - fabro-ae57 — Fix stale fabro-9467 reviewer-node comment in workflow.fabro
- basis: run 01M2Q7VVHNQPTTEY14AFPBHFBZ, workflow version f451989b6dd39304d6e41db1810163069ec25f26057a791fc0c7397cc1e94c00, commit dac906d1f192a4ef93d05424770f3b4aed0dde03
- revised_at_commit: dac906d1f192a4ef93d05424770f3b4aed0dde03 (ADR-0015: engine drift signal for later judgement)

## Findings

### Planner compile-probe cost tier (filed: fabro-6ae9)
Add probe cost-tier guidance to `.fabro/workflows/develop/prompts/planner.md`: grep/parse-level verification before build-level probes, `timeout_ms >= 600000` on compile/test shells, never `cargo run` cold. Expected effect: eliminates the 180s-cold `cargo run -- parse` timeout class (188s wasted, ~34% of run wall, planner at 78% of run cost in the subject run). Not a duplicate: closed implementer-side rules (fabro-e702, fabro-3c6c) never touched planner.md; open probe seeds cover other files/mechanisms.

### Stale reviewer-node comment (filed: fabro-ae57)
Correct the `preamble_inline_max_kb` comment in the reviewer node block of `.fabro/workflows/develop/workflow.fabro` (still describes a tool-less prompt node). Expected effect: removes a recurring LLM-noticed contradiction that forces a planner waiver every lap; verification-only passes cannot fix comments under the report-don't-fix rule. Not a duplicate: fabro-9467 (closed) delivered the feature; fabro-a512/fabro-59a0 target different comments.
