# Revision — run 01M1XVXF3MJHD0DXVCA7YXDH9G

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M1XVXF3MJHD0DXVCA7YXDH9G.md
- seeds filed: fabro-513e (just validate-workflows recipe), fabro-43ba (transient churn annotation in evidence.nu), fabro-fb19 (planner briefs inline engine facts), fabro-21c0 (workaround-is-a-painpoint clause in implementer prompt), fabro-59b0 (mechanical-edit rule for comment blocks + Evidence node phrase fix)
- basis: run 01M1XVXF3MJHD0DXVCA7YXDH9G, workflow version 8afe037fc8c61680097f808a27f66ee20da8b546373f6e6e69ad4a09aee94a98, commit 9655d15457db349b4920de1f37eb76338e43eab9
- revised_at_commit: 9655d15457db349b4920de1f37eb76338e43eab9 (ADR-0015: engine drift signal for later judgement)

## Findings

### Add a just validate-workflows recipe for graph-only validation without a Rust cold build
- filed: fabro-513e (priority 1)
- Change: `just validate-workflows` recipe (justfile + small nu wrapper around fabro-validate) that resolves workflow-relative `@`-refs or scopes out `unresolved_file_ref`, so pure graph edits verify in seconds.
- Expected effect: eliminates the ~7.1-min cold build and the temp-test file dance (427s of the 445s implementer tool time, ~25% of run wall); implementer wall drops ~30% on platform-targeting seeds.

### Mark no-net-diff churn entries as transient in the develop evidence capture
- filed: fabro-43ba (priority 2)
- Change: `.fabro/workflows/develop/scripts/evidence.nu` annotates churn entries whose net diff vs the claim base is empty with "(transient, no net diff)" or drops them.
- Expected effect: removes the reviewer's manual `git diff` fallback on `.seeds/issues.jsonl` and the risk of a Verification-blocked cycle. Complements fabro-93a7 (seed-work classification); different mechanism from fabro-1e9f (diff base) — cross-referenced, nothing closed.

### Require planner briefs to inline load-bearing engine facts instead of citing source paths
- filed: fabro-fb19 (priority 2)
- Change: `.fabro/workflows/develop/prompts/planner.md` step 5 gains a brief-shaping rule: when a brief depends on verified engine mechanics, inline 2–3 exact facts (attribute names + constraints like brace-free stderr, `output.<node_id>` convention).
- Expected effect: implementer re-exploration replaced by inlined facts; direct cut into the costliest stage (86% of run cost, $1.084 of $1.256).

### Port the workaround-is-a-painpoint clause to the implementer prompt's journal section
- filed: fabro-21c0 (priority 2)
- Change: `.fabro/workflows/develop/prompts/implementer.md` Journal section adds the reviewer prompt's clause "a workaround you performed is a painpoint, not an observation".
- Expected effect: cost-bearing friction reliably lands as painpoints; fix ideas like fabro-513e get filed at correct priority without a human rereading observations.

### Extend the mechanical-edit rule to comment blocks and fix the duplicated phrase in the Evidence node comment
- filed: fabro-59b0 (priority 2)
- Change: fix the one-word duplication in `.fabro/workflows/develop/workflow.fabro` (Evidence node comment) with the next loop-touching seed; extend `.fabro/workflows/develop/prompts/implementer.md` step 4 so comment-block rewrites get the same one-pass sed + grep-anchor check.
- Expected effect: recurrence of hand-edit slips in large comment rewrites prevented at negligible cost. Closed seed fabro-37a6's mechanical-transform rule covers code only — complementary, nothing superseded.
