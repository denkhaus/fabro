# Revision — run 01M1YGJ7JSWAVWJT45DRDZ89Z6

- status reviewed: succeeded
- review: `.fabro/reviews/develop/01M1YGJ7JSWAVWJT45DRDZ89Z6.md`
- seeds filed: fabro-8275 — Downgrade the by-design preamble allow-key absence warn to info
- basis: run 01M1YGJ7JSWAVWJT45DRDZ89Z6, workflow version 4ff76f32665f954e9a8e2027ec2a4a5b48266707730ae0620fcfd3f79acd4a10, commit f7126d9612d28110fca518d3aeeeb941bd7b659d
- revised_at_commit: f7126d9612d28110fca518d3aeeeb941bd7b659d (ADR-0015: engine drift signal for later judgement)

## Findings

### Downgrade the by-design preamble allow-key absence warn to info
- filed: fabro-8275
- Change: in the preamble fidelity module of `fabro-workflow`, emit info (not warn) when an allow-listed context key is absent on a node's first visit; keep warn for repeat visits or genuine contract regressions. Evidence: the warn fired on a green first-visit stage of this run even though absence on first visit is by design.
- Expected effect: warn-level logs stop crying wolf on every green run while real fidelity regressions stay visible.
- Dedup note: searched `preamble_allow_keys` and related themes; no seed covers the log-level change (`fabro-meta-c454` targets the input-contract closure, cross-referenced in the description — thematic overlap, not supersession).
