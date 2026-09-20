# Revision — run 01M2YSY6EGPS2G5ZG8FS3AG0VZ

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2YSY6EGPS2G5ZG8FS3AG0VZ.md
- seeds filed: none — healthy run, zero balance credit (no same-pass stale/superseded closes)
- balance: 0 non-exempt seeds filed / 0 — no credit this pass
- basis: run 01M2YSY6EGPS2G5ZG8FS3AG0VZ, workflow version d5fd86fe540c675db0ac8a3593641edecf71f03c37da78d94f27861d96a54d94, commit dfdb92cfc0dc95b16ba0a214284878ae68f53831
- revised_at_commit: dfdb92cfc0dc95b16ba0a214284878ae68f53831 (ADR-0015: engine drift signal for later judgement)

## Findings

### Park upstream-blocked seeds fabro-af22 and fabro-23d4 out of `sd ready` triage
- overflow-dup: Park upstream-PR-gated seeds mechanically in the planner preflight (open in 01M2VVFMSQXHK0E3SNV2GNTP43.md, line 14) — same theme (parking upstream-gated fabro-af22 out of triage) already an open overflow via a different mechanism (preflight arm + seed body gate line vs. blocker-seed filing via `sd dep add`); the fabro-23d4 half additionally overlaps the open out-of-repo-target overflow in 01M2YMRJQZMC6SVTQJ3JC49CWC.md line 16. Not filed: zero credit this pass.

### Mandate a mechanical seed-body re-emit with Basis-survival assert in `planner.md` step 3
- overflow: Mandate a mechanical seed-body re-emit with Basis-survival assert in `.fabro/workflows/develop/prompts/planner.md` step 3 — when the planner corrects a seed body via `sd update --description`, require the round-trip: extract description from `sd show <id> --format json`, append the correction via heredoc, feed `sd update` from the file, then one grep assert that the `Basis:` line survived byte-identical; effect: prevents the hand-retyped-body corruption seen at event seq 83 (fabro-23d4 Basis hash) that cost a full recovery LLM round (~25 s / $0.04). Distinct from open fabro-19df (docs recipe for reason-closes) and closed fabro-c0a8 (when-to-correct rule). No open overflow covers this theme (checked the ledger listing this pass).

### Qualify the fs_hide loop-asset bullet in `project-facts.md` per node
- overflow-dup: Fix the PROJECT_FACTS fs_hide bullet: the reviewer node is exempt by design (open in 01M2Y8QYY0QY3Q4Z8DVA2K0YMX.md, line 13) — same theme, same concrete edit (per-node qualification of the shared fs_hide bullet: binds planner/implementer, reviewer exempt by design). Not filed: zero credit this pass.

### Allowlist intentional routing-named fields in `prompt-lint.nu` schema warnings
- overflow-dup: Silence by-design prompt-lint warnings in the qualitygate (open in 01M2XQJC3TWRMJ1128WRXRQYXH.md, line 20) — same theme (allowlisting routing-intent schemas so the ~10-11 per-gate warnings on `planner-output.schema.json` / `develop-output.schema.json` stop riding into reviewer preambles); the qualitygate-allowlist and the `prompt-lint.nu` allowlist/downgrade are complementary mechanisms for the same noise class. Not filed: zero credit this pass.
