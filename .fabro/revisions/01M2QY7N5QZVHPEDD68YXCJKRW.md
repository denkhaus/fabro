# Revision — run 01M2QY7N5QZVHPEDD68YXCJKRW

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2QY7N5QZVHPEDD68YXCJKRW.md
- seeds filed: fabro-048a — Implementer Blocked template: omit preferred_next_label from the non-emittable route; fabro-2afd — Planner degraded-mode: mandate dup-run-check when output.preflight is absent
- basis: run 01M2QY7N5QZVHPEDD68YXCJKRW, workflow version f4ab9ff48b6e162ee81f42689c29e27253fba4fe3632992c2e164afa31eb5ca5, commit 1b5dc71678a45ede65e175a2b60d31fbd90755d3
- revised_at_commit: 1b5dc71678a45ede65e175a2b60d31fbd90755d3 (ADR-0015: engine drift signal for later judgement)

## Findings

### Implementer Blocked template: omit preferred_next_label from the non-emittable route
- filed: fabro-048a
- Change: in `.fabro/workflows/develop/prompts/implementer.md`, drop `preferred_next_label` from the Blocked-route JSON example — the implementer→planner edge fires on outcome=failed and carries no such label. Effect: every duplicate-bounce Blocked route saves one ~11s/$0.009 validation-retry round (evidence: events seq 94–99 of this run). Not a duplicate of fabro-dd22 (revisor analyze.md) or fabro-53d3 (new Blocked edge).

### Planner degraded-mode: mandate dup-run-check when output.preflight is absent
- filed: fabro-2afd
- Change: in `.fabro/workflows/develop/prompts/planner.md`, the ALREADY-LANDED arm must mandate `nu .fabro/scripts/dup-run-check.nu <id> --self <run-id>` per candidate with base-history refs whenever `output.preflight` is absent (preflight dead-lands per fabro-2ade), instead of eyeballing commit subjects. Effect: one deterministic planner lap resolves duplicates instead of three disagreeing LLM laps (~63s/$0.085 lost here; wrongful close of fabro-fb06). Complementary to fabro-2ade/fabro-a32f; cross-referenced, nothing closed.
