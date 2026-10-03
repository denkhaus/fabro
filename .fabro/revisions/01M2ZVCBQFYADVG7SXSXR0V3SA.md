# Revision — run 01M2ZVCBQFYADVG7SXSXR0V3SA

- status reviewed: failed
- review: .fabro/reviews/develop/01M2ZVCBQFYADVG7SXSXR0V3SA.md
- seeds filed: none — zero filing credit this pass (ADR-0022: no same-pass stale/superseded closes; 4 surviving findings journaled below)
- balance: 0 non-exempt seeds filed / 0 — no credit this pass
- basis: run 01M2ZVCBQFYADVG7SXSXR0V3SA, workflow version e3c92fec9a9a6fd9256277d60e06e12dddcacf6779280c9df5c2b2825b813ea8, commit 869d6a8594556fe3b11c9b6d18e9d86e9d6ceb73
- revised_at_commit: 869d6a8594556fe3b11c9b6d18e9d86e9d6ceb73 (ADR-0015: engine drift signal for later judgement)

## Findings

### Make claim-check.nu seed-id validation portable: generic `<project>-<hex4>` pattern instead of the hardcoded fabro- prefix

- verdict: overflow (no balance credit this pass)
- change: in `.fabro/workflows/develop/scripts/claim-check.nu` (line 21 and header line 10) replace `str starts-with "fabro-"` with `$seed_id =~ '^[a-z][a-z0-9]*-[0-9a-f]{4}$'`
- dedupe: `sd search` on claim-check / seed-id found only fabro-846b (slash-lint) and fabro-4c81 (path resolution); no seed covers the id-prefix check
- effect: the run-killer class (run 01M2ZVCBQFYADVG7SXSXR0V3SA planner's valid `seeds-e218` failed claim_check deterministically 3x until the breaker killed the run at 133s/$0.249) disappears on every ported line

- overflow: Make claim-check.nu seed-id validation portable — replace `str starts-with "fabro-"` with `$seed_id =~ '^[a-z][a-z0-9]*-[0-9a-f]{4}$'` at `.fabro/workflows/develop/scripts/claim-check.nu` lines 10/21; effect: deterministic claim_check run-killer on non-fabro-prefixed seed ids disappears, planner routes to implementer instead of breaker death

### Add a soft exit from claim_check for deterministic failures instead of re-entering the LLM planner

- verdict: overflow (no balance credit this pass)
- change: in `.fabro/workflows/develop/workflow.fabro` line 522, add `claim_check -> exit [kind="soft", condition="failure_class=deterministic"]` mirroring the reviewer/closeout soft exits (lines 563/570), keeping the re-plan edge for non-deterministic failures
- dedupe: no existing seed names this edge change (searches on claim-check, schema, preflight)
- effect: deterministic contract failures park in under 1s/$0 instead of ~75s/$0.14 (run 01M2ZVCBQFYADVG7SXSXR0V3SA: planner visits 2+3 cost $0.143 = 57.6% of run cost for zero effect, planner@2 had already diagnosed the exact bug)

- overflow: Soft exit from claim_check on deterministic failures — add `claim_check -> exit [kind="soft", condition="failure_class=deterministic"]` in `.fabro/workflows/develop/workflow.fabro` (keeping the re-plan edge for non-deterministic drops per fabro-c42f); effect: deterministic contract failures park in <1s/$0 instead of ~75s/$0.14 of wasted planner re-entry

### Treat engine-breaker-latched failed run tips as terminal immediately in planner-preflight and tracker-guard

- verdict: overflow-dup: Tracker guard: requeue immediately when the claiming run is terminal-failed with no PR (open in .fabro/revisions/01M2YAJMSPY766DS0AGW1859CQ.md) — same failed-run-terminality theme in `tracker-guard.nu`; this pass adds the planner-preflight `terminal-tip?` grace-window arm (latch-latched failed tips terminal now, bypassing the 60min grace and the closeout-record requirement), distinct from open fabro-ab93 (PR-state gating) and closed fabro-d9f7 (age-based requeue)

### Pin the current_seed_id format in planner-output.schema.json with the fabro-017f teeth pattern

- verdict: overflow (no balance credit this pass)
- change: in `.fabro/workflows/develop/schemas/planner-output.schema.json` add `"current_seed_id": { "type": "string", "pattern": "^[a-z][a-z0-9]*-[0-9a-f]{4}$" }` under context_updates.properties, mirroring the gate-command-ban pattern on `current_seed_brief`
- dedupe: no existing seed covers schema-format invariants for the seed id
- effect: a malformed id burns one output_retries round at the planner in seconds, before the sd claim, instead of a post-claim deterministic crash (complements the claim-check portability fix above)

- overflow: Pin current_seed_id format in planner-output.schema.json — add pattern `^[a-z][a-z0-9]*-[0-9a-f]{4}$` on `current_seed_id` under context_updates.properties in `.fabro/workflows/develop/schemas/planner-output.schema.json`; effect: malformed seed id caught pre-claim at the planner in seconds instead of a post-claim deterministic graph cycle
