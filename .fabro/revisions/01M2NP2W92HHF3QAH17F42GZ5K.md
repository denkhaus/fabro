# Revision — run 01M2NP2W92HHF3QAH17F42GZ5K

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2NP2W92HHF3QAH17F42GZ5K.md
- seeds filed: fabro-39ce — Planner: hand root-cause findings to the implementer as labeled unverified hypotheses; fabro-a158 — Implementer: count matches before any pattern-wide sed/perl transform; fabro-9f4a — Automation store: validate environment existence in replace with a typed error
- basis: run 01M2NP2W92HHF3QAH17F42GZ5K, workflow version ed2ea157f5427586f9407c99360442db1db24ab8f232704a852f89d8823165db, commit 2f89518ca990f96a550a908add8f685dd2d3785b
- revised_at_commit: 2f89518ca990f96a550a908add8f685dd2d3785b (ADR-0015: engine drift signal for later judgement)

## Findings

### Planner: hand root-cause findings to the implementer as labeled unverified hypotheses
- filed: fabro-39ce
- Change `.fabro/workflows/develop/prompts/planner.md` step 6: when basis verification reveals a likely mechanism, add a bullet labeled `unverified hypothesis (planner observation)` to the brief, never as a requirement. Expected effect: implementer exploration starts at the suspected mechanism on bug seeds — the planner's discarded fabro-b959 root cause cost the run's dominant stage 87.6% of cost ($1.471 of $1.68) re-deriving it.

### Implementer: count matches before any pattern-wide sed/perl transform
- filed: fabro-a158
- Change `.fabro/workflows/develop/prompts/implementer.md` step 4: require `rg -c` match counts before any pattern-wide sed/perl transform; if the count exceeds intended sites, use line-addressed sed or unique-context edit_file. Expected effect: removes the collateral-rewrite → false-failure → diagnosis-detour chain observed in this run's dominant stage.

### Automation store: validate environment existence in replace with a typed error
- filed: fabro-9f4a
- Change the store-layer replace path in `lib/components/fabro-automation/src/store.rs` to validate environment existence and return a typed validation error mirroring the handler's `resolve_automation_environment` check. Expected effect: actionable errors for every store caller; removes the FK-code-787 reproduction detour class.
