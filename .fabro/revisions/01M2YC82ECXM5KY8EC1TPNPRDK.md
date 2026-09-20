# Revision — run 01M2YC82ECXM5KY8EC1TPNPRDK

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2YC82ECXM5KY8EC1TPNPRDK.md
- seeds filed: none — zero balance credit this pass (no same-pass stale/superseded closes)
- balance: 0 non-exempt seeds filed / 0 — no credit this pass
- basis: run 01M2YC82ECXM5KY8EC1TPNPRDK, workflow version bdd056c377ba821c9d6cf0cd764c7379961019561cf7e1151acf0b7bec5da4ad, commit 85abe7dda9d914bce6f122389a5a739b95c5e39a
- revised_at_commit: 85abe7dda9d914bce6f122389a5a739b95c5e39a (ADR-0015: engine drift signal for later judgement)

## Findings

### Park fabro-af22 as blocked-external until upstream PR #784 merges
- overflow-dup: Park upstream-PR-gated seeds mechanically in the planner preflight (open in 01M2VVFMSQXHK0E3SNV2GNTP43.md) — that ledger entry already carries the `sd update fabro-af22` tracker action (gate line on the seed body) beside the preflight arm; same theme, no second open entry.
- filed id: none — overflow-dup (zero credit); the consuming pass files the ledger entry which subsumes this action

### Add an external_block arm to planner-preflight.nu for seeds parked behind open upstream PRs
- overflow-dup: Park upstream-PR-gated seeds mechanically in the planner preflight (open in 01M2VVFMSQXHK0E3SNV2GNTP43.md) — same mechanism (preflight arm flagging upstream-PR-gated seeds so the planner skips for free); this finding's `UPSTREAM PR OPEN` body-marker detail merges into that entry at filing time.
- filed id: none — overflow-dup (zero credit); NEXT PASS: re-run dedupe, then file the ledger entry

### Authoring lint in validate-workflows.nu: every output.* allow-key must resolve to a command-node id
- overflow: Authoring lint in validate-workflows.nu: every output.* allow-key must resolve to a command-node id — add a gate-red check to `.fabro/scripts/validate-workflows.nu` that every `output.<x>` entry in any node's `context_allow_keys`/`preamble_allow_keys` resolves to a declared command-node id (or an agent node's own dedup key); effect: the inert-enrichment/blind-stage class (closed fabro-7028's `output.gate_known_bug_hits`, this run's engine warn seq 99 `context_allow_keys dropped: output.planner`) is caught at authoring time instead of after days of live warns. Dedupe re-checked this pass (sd searches on validate-workflows / allow-keys): closed fabro-8bf4 is the stage-runtime unemitted-key lint, open fabro-a341 tunes that lint's tolerance — neither is the authoring-time graph cross-check; adjacent open overflow in 01M2WCNY3WGP4QFZ24KPACBTND.md fixes the planner node's own allow-key (graph edit, different mechanism), cross-reference it when filed.
- filed id: none — overflow (zero credit); NEXT PASS: re-run dedupe, then file

### Suppress prompt-lint routing-named warnings for schemas that declare intentional routing intent
- overflow-dup: Silence by-design prompt-lint warnings in the qualitygate (open in 01M2XQJC3TWRMJ1128WRXRQYXH.md) — same theme: allowlist routing-intent schemas (`planner-output.schema.json`, conductor schema) in prompt-lint; this finding adds the explicit marker-or-description mechanism detail, merging at filing time.
- filed id: none — overflow-dup (zero credit); NEXT PASS: re-run dedupe, then file the ledger entry
