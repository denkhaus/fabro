# Revision — run 01M0T9B7T6XNM1D7JNY35Y6H8K

- status reviewed: failed
- review: .fabro/reviews/develop/01M0T9B7T6XNM1D7JNY35Y6H8K.md
- seeds filed:
  - fabro-4cd8 Render verification report only in implementer JSON summary
  - fabro-fbed Add deterministic post-approval seed-close node
  - fabro-750c Include gate stdout tail in evidence capture
  - fabro-35ab Raise develop preamble budget to 20 KB
  - fabro-1399 Add sd syntax cheat-sheet and call-order rule to planner prompt
  - fabro-4881 Gate implementer sd show re-fetch on brief quality

## Findings

### Render verification report only in implementer JSON summary
Filed as fabro-4cd8. In `.fabro/workflows/develop/prompts/implementer.md`, state the per-criterion PASS/FAIL report lives only in the JSON `implementation_summary` and pre-JSON text stays one short paragraph. The implementer duplicated the report as markdown prose plus verbatim JSON (8,024 of 10,693 output tokens; 60% of run cost), and both copies were re-read as preamble by reviewer and planner. Expected effect: ~10-15% lower run cost and smaller downstream preambles.

### Add deterministic post-approval seed-close node
Filed as fabro-fbed. In `.fabro/workflows/develop/workflow.fabro`, insert a command node on the reviewer Approved edge that closes the seed, checks `sd ready`, and routes empty-tracker to exit vs seeds-remaining to planner. In this run planner@2 only mechanically closed fabro-8208 and listed an empty tracker (14.8s, $0.016). Expected effect: one fewer LLM visit per seed cycle and no misparse risk on the tracker-critical path.

### Include gate stdout tail in evidence capture
Filed as fabro-750c. In `.fabro/workflows/develop/scripts/evidence.nu`, append the quality gate's small stdout tail to the evidence integrity header. tester@1 output was 288 bytes yet reached the reviewer only as a blob ref, so reviewer@1 re-verified live (six re-runs, 97.5s, 25% of run cost). Expected effect: removes the reviewer's main excuse for tool re-verification and shrinks the Verification-blocked failure class.

### Raise develop preamble budget to 20 KB
Filed as fabro-35ab. In `.fabro/workflows/develop/workflow.fabro`, raise graph attr `preamble_budget_kb` from 12 to about 20. Worker logs show the preamble budget exceeded even after demotion at four consecutive prompts (up to 18,053 bytes vs 12,288 budget), firing blob-ref demotion every cycle. Expected effect: fewer blob-ref round-trips and no per-cycle budget warnings. Complements fabro-83f9 (reviewer-node budget).

### Add sd syntax cheat-sheet and call-order rule to planner prompt
Filed as fabro-1399. In `.fabro/workflows/develop/prompts/planner.md`, document exact write syntax (`sd update <id> --status in_progress`, `sd close <id>`, no `--format` on writes) and instruct `sd ready` first with `sd list` only when ready is empty or blockers matter. planner@1 wasted a call on `sd update --format json` (event seq 62-63) and ran redundant parallel `sd ready` and `sd list`. Expected effect: 1-2 fewer tool calls and one fewer LLM round-trip per planning visit.

### Gate implementer sd show re-fetch on brief quality
Filed as fabro-4881. In `.fabro/workflows/develop/prompts/implementer.md`, make the `sd show` re-read conditional: only when the brief is thin, ambiguous, or verification-only. The prompt called the brief authoritative yet numbered step 1 ordered re-reading the seed (event seq 94-98), risking re-import of ambiguity the planner had settled. Expected effect: one fewer call and turn per cycle, planner's resolved reading stays authoritative.

### Routed as platform painpoints (not filed as seeds, per repo policy)
- Publish-auth: PAT lacks PR scope despite declared `integrations.github.permissions`; PR creation 403 after 7 minutes of green work. Fix: PR-capable credential or preflight probe plus JSON guard on PR-body generation.
- Publish-failed UX: engine marks publish-failed runs plain failed with 6-of-5 stage progress. Fix: distinct recoverable `publish_failed` terminal state with retry-publish, and unique-node progress denominator. (Partially tracked by fabro-6a77/fabro-45bf/fabro-696c.)
