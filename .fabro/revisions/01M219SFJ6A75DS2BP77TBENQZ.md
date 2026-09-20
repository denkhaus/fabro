# Revision — run 01M219SFJ6A75DS2BP77TBENQZ

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M219SFJ6A75DS2BP77TBENQZ.md
- seeds filed: fabro-e33d — Add closest-match hint to edit_file errors and pin old_string sourcing in implementer prompt
- basis: run 01M219SFJ6A75DS2BP77TBENQZ, workflow version 84429d0a3253eaa52e2406a04a3b97b4365d0ff68b0cdc2f46c296e8c05ec7a6, commit aed06d18be94f10f758651dcaf0eadb97f5d0c17
- revised_at_commit: aed06d18be94f10f758651dcaf0eadb97f5d0c17 (ADR-0015: engine drift signal for later judgement)

## Findings

### Add closest-match hint to edit_file errors and pin old_string sourcing in implementer prompt
- filed: fabro-e33d
- change: (a) include the line number of the closest fuzzy match / first diverging whitespace in the edit_file `old_string not found` error (`lib/components/fabro-agent/src/tools.rs:234`); (b) add one implementer-prompt line: build old_string from raw `sed -n` output, never from the line-numbered read_file rendering.
- effect: removes one diagnostic plus one LLM round per mismatch and prevents false "transient" conclusions from being journaled as durable lore.
- dedupe check: no duplicate; fabro-645d (scoped reads via `sed -n` for line anchors) is complementary and cross-referenced in the seed; fabro-ee2c (ml record gating for near-miss lessons) covers a different mechanism.
