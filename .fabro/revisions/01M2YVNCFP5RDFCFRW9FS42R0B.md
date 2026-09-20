# Revision — run 01M2YVNCFP5RDFCFRW9FS42R0B

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2YVNCFP5RDFCFRW9FS42R0B.md
- seeds filed: none — zero balance credit this pass (ADR-0022)
- balance: 0 non-exempt seeds filed / 0 — no credit this pass
- basis: run 01M2YVNCFP5RDFCFRW9FS42R0B, workflow version 8d3a780e5a0665da9cb87b36306f1ef7e10841bcc767996f563f828c08a277b6, commit 8eaa881fd2d86e0e289dce3a4d40ce2e5ebf8ee1
- revised_at_commit: 8eaa881fd2d86e0e289dce3a4d40ce2e5ebf8ee1 (ADR-0015: engine drift signal for later judgement)

## Findings

### 1. Audit workspace for root-blind permission-based tests (chmod void under uid 0)

- filed: overflow to journal (no balance credit)
- concrete change: audit `lib/` for other `#[cfg(unix)]` permission-based tests relying on chmod taking effect; make them skip or assert differently under uid 0. Run evidence: this run's only gate red (tester@1: 2680/2681) was pre-existing `publish_failure_before_flip_leaves_old_bundle_consistent` — chmod 0o000 setup void under uid 0, deterministic red in run containers while green on dev hosts; it forced a full extra cycle (~7.1 min: tester@1 99.8s + gatebounce + implementer@2 274s/$0.21 + tester@2 52.3s) and consumed one of three gate cycles.
- dedupe: `sd search "chmod"` empty; fabro-1486, fabro-ca1f, fabro-dd29 are different mechanisms; no open overflow in the ledger covers this theme.

- overflow: Audit workspace for root-blind permission-based tests (chmod void under uid 0) — audit `lib/` for `#[cfg(unix)]` permission-based tests relying on chmod taking effect and make them skip or assert differently under uid 0; effect: no pre-existing deterministic-red tests blocking seed cycles.

### 2. Require self-contained descriptions on implementer ml record capture

- filed: overflow to journal (no balance credit)
- concrete change: add one line to the lesson-capture section of `.fabro/workflows/develop/prompts/implementer.md` mandating a self-contained description (pattern plus when to apply) on every `ml record` call, rejecting placeholder text. Run evidence: lesson record mx-490000 (`ttft-timeout-at-stream-middleware-seam`) landed with the literal description "verify id"; the reviewer journaled the empty body as a painpoint.
- dedupe: existing ml seeds (fabro-17df when-to-record, fabro-8d81 mx-id recovery, fabro-96bd flag contract, fabro-c271 quoting hazard) cover different mechanisms; closed fabro-e702 bans placeholder appends in the implementer prompt, not ml record descriptions; `sd search "placeholder"` and `"description"` show no seed naming this change; no open overflow in the ledger covers this theme.

- overflow: Require self-contained descriptions on implementer ml record capture — one line in the lesson-capture section of `.fabro/workflows/develop/prompts/implementer.md` mandating a self-contained description (pattern + when to apply) on every `ml record` call, rejecting placeholder text; effect: expertise records stay useful to `ml search` instead of starving.
