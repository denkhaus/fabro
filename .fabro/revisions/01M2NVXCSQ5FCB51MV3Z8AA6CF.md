# Revision — run 01M2NVXCSQ5FCB51MV3Z8AA6CF

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2NVXCSQ5FCB51MV3Z8AA6CF.md
- seeds filed:
  - fabro-534e — Closeout: re-file unmet user-action arms as open seeds before sd close
  - fabro-e395 — Engine: per-call first-token (TTFT) timeout with transparent retry for stage LLM calls
- basis: run 01M2NVXCSQ5FCB51MV3Z8AA6CF, workflow version ed2ea157f5427586f9407c99360442db1db24ab8f232704a852f89d8823165db, commit b6cbc1ef6355b619ca9cfc449e39bc19371b00fd
- revised_at_commit: b6cbc1ef6355b619ca9cfc449e39bc19371b00fd (ADR-0015: engine drift signal for later judgement)

## Findings

### Closeout: re-file unmet user-action arms as open seeds before sd close
- filed: fabro-534e
- Concrete change: `.fabro/workflows/develop/scripts/closeout.nu` (or planner pre-approval in `prompts/planner.md`) must, when a brief carries an EXEMPTION bullet, file that arm as a new open OPS seed before `sd close`.
- Expected effect: deferred human actions stay tracker-visible instead of dying with the closed run's journal (fabro-79d8's deploy arm of the b959 fix remains unmet otherwise).
- Not a duplicate: fabro-ae74 covers structured arms at intake; closed fabro-02c4 covers closure discipline; neither re-files unmet arms at closure.

### Engine: per-call first-token (TTFT) timeout with transparent retry for stage LLM calls
- filed: fabro-e395
- Concrete change: engine-side per-call TTFT timeout with transparent retry/fallback in the stage LLM client, near `lib/components/fabro-workflow/src/model_fallback.rs`.
- Expected effect: bounds stage latency against provider stalls — 163 s first-token wait (67% of the 242 s planner stage) would have been cut.
- Not a duplicate: fabro-83af covers stage-level retry/backoff; a hung-alive call never fails so no retry fires.
