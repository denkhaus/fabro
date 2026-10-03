# Revision — run 01M2W7GSEFRRPV48NVW69GXJD7

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2W7GSEFRRPV48NVW69GXJD7.md
- seeds filed: none — zero credit (0 same-pass stale/superseded closes); all findings journaled as overflow
- balance: 0 non-exempt seeds filed / 0 — no credit this pass
- basis: run 01M2W7GSEFRRPV48NVW69GXJD7, workflow version 8b7bbe9d58b678ad56c49416d28afbf7872db2deec867d142043b12649cefcc8, commit d2a5e6bca9bfbea7d5b4e2e0c64fc76245901d10
- revised_at_commit: d2a5e6bca9bfbea7d5b4e2e0c64fc76245901d10 (ADR-0015: engine drift signal for later judgement)

## Findings

All four findings survived analyzer dedupe but exceed this pass's filing credit (ADR-0022); each is journaled as an `overflow:` observation for the next pass to re-dedupe and file against its own balance.

### Planner: persist upstream-blocked skip verdicts into the seed body
- filed: overflow-to-journal
- Change: in `.fabro/workflows/develop/prompts/planner.md` step 3, when a top candidate is gated on an upstream/external PR, write the verdict back via the stale-spec-correction form (`sd update <id> --description "<full body> + skip note: upstream PR #784 verified Open <date>"`). Expected effect: ~40s and ~$0.05 saved per develop run while an upstream-gated seed tops the queue (run evidence: 5 tool calls across 4 LLM rounds adjudicating fabro-af22, seq 46-60).

### Resolve loop-asset-relative anchors in the planner preflight
- filed: overflow-to-journal
- Change: in `.fabro/workflows/develop/scripts/planner-preflight.nu`, when a root-joined anchor path is absent, retry resolution against `.fabro/workflows/<name>/` bases before flagging missing_file. Expected effect: false missing_file flags on loop-asset citations disappear (run evidence: fabro-60a0 flagged although `.fabro/workflows/architect/prompts/survey.md` exists, seq 21/70).

### Add preflight and claim_check to downstream preamble_stages_ignore
- filed: overflow-to-journal
- Change: in `.fabro/workflows/develop/workflow.fabro`, append `preflight,claim_check` to the `preamble_stages_ignore` of the implementer and reviewer nodes (render-only, landed via fabro-a85b). Expected effect: ~1.5KB less preamble per downstream stage and one fewer irrelevant data surface to misread.

### Exempt engine response-dedup output keys from the context-update drop lint
- filed: overflow-to-journal
- Change: engine-side, exempt the response-dedup wrapper's `output.*` keys from the context_allow_keys drop lint (or declare them implicitly per node). Expected effect: drop notices regain their meaning; a genuine drift event no longer drowns in expected noise (run evidence: benign `context_allow_keys dropped: output.planner` warn at seq 92, from the fabro-b907 wrapper).
