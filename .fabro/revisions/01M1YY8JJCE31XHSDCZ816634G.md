# Revision — run 01M1YY8JJCE31XHSDCZ816634G

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M1YY8JJCE31XHSDCZ816634G.md
- seeds filed: fabro-c1bb — Drop current_seed_brief from the develop reviewer node's preamble_allow_keys; fabro-645d — Implementer prompt: scoped reads via sed -n when the brief pins line anchors
- basis: run 01M1YY8JJCE31XHSDCZ816634G, workflow version 08846dfb574d2ff5db4e16cc712b4a81c7801307975d397a365a9bc676a3d611, commit e53f85b
- revised_at_commit: e53f85b (ADR-0015: engine drift signal for later judgement)

## Findings

### Drop current_seed_brief from the develop reviewer node's preamble_allow_keys
- filed: fabro-c1bb
- Concrete change: remove `current_seed_brief` from the reviewer node's `preamble_allow_keys` in `.fabro/workflows/develop/workflow.fabro`. The preamble duplicated the seed spec already delivered inside the evidence capture, pushing the 24 KB aggregate inline budget over and demoting the capture to a `blob-ref` (forced `read_file` detour, reviewer@1). Expected effect: capture renders inline, one model round-trip saved per review, `verification_blocked`-on-blob failure mode gone. Cross-referenced (not superseded) open seed fabro-cf3e, which raises the budget instead — different mechanism.

### Implementer prompt: scoped reads via sed -n when the brief pins line anchors
- filed: fabro-645d
- Concrete change: add scoped-read guidance to `.fabro/workflows/develop/prompts/implementer.md`: when the brief names line anchors, read with `sed -n '<range>p'` instead of whole-file `cat`. Evidence: implementer `cat -n`'d the entire 111-line `planner.md` (12,096 bytes, event seq 84–85) though the brief pinned line targets. Expected effect: ~10 KB less context per platform-file edit pass. Reviewer-side analog fabro-9a43 exists; this is a new mechanism on a different node.
