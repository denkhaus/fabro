# Revision — run 01M5K4BB81H36V7B4QHF9NTDS2

- status reviewed: failed
- review: .fabro/reviews/develop/01M5K4BB81H36V7B4QHF9NTDS2.md
- items filed: none — tracker binding outage (5th consecutive context); 0 credit this pass, all surviving findings journaled as overflow below
- balance: 0 non-exempt items filed / 0 — no credit this pass (tracker tools down; no stale/superseded closes possible)
- basis: run 01M5K4BB81H36V7B4QHF9NTDS2, workflow version absent, commit 523e320959ed6b5e63daad7be9450addd87add0e
- revised_at_commit: 523e320959ed6b5e63daad7be9450addd87add0e (ADR-0015: engine drift signal for later judgement)

## Findings

Verdict: duplicate run — fabro-d48f already closed on base branch via PR #468 (commit 9c8139300, run trailer names 01M5K208QW1S349SKQ4QJ68EVT); the target run left zero commits, died at claim_check after a 3x cycle tripped the engine breaker. No release needed (no claim id exists; journal absent).

Dropped at analyze: preflight in-flight rec — duplicate_of fabro-997d; RUN TITLE length-cap rec — duplicate_of fabro-b529. Closeout-race re-check degraded (tracker down): no item in `.seeds/issues.jsonl` cites the target run — no covered_by_closeout drops.

**1. claim-check false-blocks on negation prose** — overflow-dup

overflow-dup: Stop re-citing dead paths in correction notes (open in .fabro/revisions/01M59JSC3MKGRCH5PMR53XHADZ.md) — same target (`claim-check.nu` correction-line path handling); this run's evidence strengthens the negation-window arm (a body honestly saying "X no longer exists" can never pass the gate: `claim-check.nu:120-122` blocks the rooted missing_file path `scripts/closeout-race.nu` cited solely inside the "(Correction 2026-10-23: … no longer exists …)" parenthetical).

**2. No verify step between --set-description and re-claim** — overflow

- overflow: Verify seed body corrections before re-claiming — in `.fabro/workflows/loop/prompts/planner.md` claim section, after any `seeds update --set-description` mandatorily re-run `seeds show <id> --format json` and confirm the stale-path list is empty before routing "Seed claimed"; effect: silent no-ops surface in seconds at the planner instead of costing a planner→claim_check round trip (~60-70s, ~$0.07-0.09 each; two wasted this run — planner@2/@3 journals asserted "body corrected" while `seeds show` stayed byte-identical at 3081 bytes across all three failures).

**3. Terminal failure payload is a 3KB stderr blob** — overflow

- overflow: Compact the claim-check failure payload — in `.fabro/workflows/develop/scripts/claim-check.nu` main, on failure emit a compact routing JSON to stdout (outcome failed, `failure_class: stale_seed_body`, blocking path list, seed id) with the full body as a bounded tail, and add `output_schema="routing"` to the `claim_check` node in loop `workflow.fabro`; effect: run reason / Slack reads "stale body fabro-d48f: scripts/closeout-race.nu" instead of a 3KB mangled body, humans triage without opening the run, and the breaker signature becomes short and exact.

**4. Labeled terminal edge for the claim-contract cycle** — overflow

- overflow: Cap the claim-contract cycle with a labeled deadlock edge — in loop `workflow.fabro` add `claim_check -> exit [x.kind="deadlock", condition="nodes.claim_check.generation >= 2", label="Claim contract deadlock"]` mirroring the tester/reviewer convention; effect: same fail-fast outcome but a labeled, resumable deadlock instead of a raw engine `workflow_error`, one fewer wasted planner pass than the breaker's 3-limit (closed fabro-51ad fixed generation conditions, so the edge now fires).

**5. Pin journal array shape in the planner output contract** — overflow

- overflow: Pin the journal array shape in the planner output schema — in `.fabro/workflows/loop/prompts/planner.md` output template (~lines 82-94), journal values are arrays of `{"text": "..."}` objects only, never bare arrays, mirrored in develop's planner.md; effect: removes one repair turn per claiming pass (planner@2's first emission was rejected "invalid JSON: key must be a string at line 8 column 624"). Complements open fabro-017f (journal key presence) — different mechanism, cross-reference when filed.
