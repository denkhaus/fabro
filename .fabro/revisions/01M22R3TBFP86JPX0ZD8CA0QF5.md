# Revision — run 01M22R3TBFP86JPX0ZD8CA0QF5

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M22R3TBFP86JPX0ZD8CA0QF5.md
- seeds filed:
  - fabro-d19e — Add a nushell-scripts skill so the implementer stops re-deriving nu 0.115 semantics
  - fabro-1314 — Planner briefs must prescribe outcomes, not unverified language mechanics
- basis: run 01M22R3TBFP86JPX0ZD8CA0QF5, workflow version 57bfc82454a2182c84939466acec9710916f642ceb8b5d804d46e80c2a30150a, commit 75c5220e994bae4fe63dd4ecbaf19f7f6fbbab98
- revised_at_commit: 75c5220e994bae4fe63dd4ecbaf19f7f6fbbab98 (ADR-0015: engine drift signal for later judgement)

## Findings

### nushell-scripts skill (filed fabro-d19e)

Implementer burned three probe turns on nu 0.115 semantics (`complete` external-only, `source` const resolution, `def main` collision, `$"…"` prose parse failure) that mx-84a287 already documents but nothing injects. Fix: `.fabro/skills/nushell-scripts/SKILL.md` plus a load-when-`.nu` line in `.fabro/workflows/develop/prompts/implementer.md` step 2. Expected: 1-2 min and $0.03-0.08 saved per script seed, fewer failed writes. Complements fabro-ee2c; not a duplicate of fabro-81b7.

### Planner outcomes over mechanisms (filed fabro-1314)

Brief criterion (3) prescribed `do { ... } | complete`, which errors in nu 0.115 on value-returning blocks; the implementer deviated and journaled it. Fix: `.fabro/workflows/develop/prompts/planner.md` step 7 — acceptance criteria as outcomes, code-snippet prescriptions marked advisory unless verified. Distinct from fabro-cf76/fabro-b8ed/fabro-9e49.
