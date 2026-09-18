# Revision — run 01M2VDR9YH2T6PFCEWJ7E3B0KS

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2VDR9YH2T6PFCEWJ7E3B0KS.md
- seeds filed:
  - fabro-f531 — Guard rg against combined -r and -n with a hard-erroring toolchain wrapper (capability-affecting, needs-user)
  - fabro-27bb — Line-watch the ADR-0022 filing balance one week post-landing
- basis: run 01M2VDR9YH2T6PFCEWJ7E3B0KS, workflow version 58a8171073f77ad9a1b6be42ffc1c46e128578a9b9875aa1a3fd2e4b3aa28c66, commit c4e6429761b665604caaf8d9f83dd0d9bfd90713
- revised_at_commit: c4e6429761b665604caaf8d9f83dd0d9bfd90713 (ADR-0015: engine drift signal for later judgement)

## Findings

**Guard rg against combined -r and -n with a hard-erroring toolchain wrapper** — filed as fabro-f531. Evidence: implementer journal records an exploratory `rg -rn "arms"` garbling to literal 'n' despite the landed prompt fix (fabro-2eb6); prose discipline has failed twice. Change: a tiny rg wrapper in `.fabro/Dockerfile.toolchain` that hard-errors on combined `-r`+`-n`, riding the image rebuild already slated by fabro-c643. Effect: the misdiagnosis class becomes mechanically impossible and stays loud even when the provider drops tool-result error flags (8x `unsupported_control` warnings this run). Capability-affecting (ADR-0019): modifies a tool on an agent-reachable surface — labeled `needs-user,revision`, implementation awaits explicit user approval.

**Line-watch the ADR-0022 filing balance one week post-landing** — filed as fabro-27bb. Evidence: the run shipped ADR-0022 filing balance (fabro-c77a, PR #248) and its implementer journal predicts `balance: 0 / 0` with overflow observations for zero-credit revisor passes. Change: re-measure revisor creates:closes around 2026-09-25 and escalate if consecutive revisor passes file zero non-exempt seeds while overflow accumulates. Effect: the loop notices backlog starvation early instead of discovering an empty tracker weeks later. Thematic overlap with fabro-4c02 (stale-evidence-gate starvation) is complementary — different mechanism, cross-referenced in the seed description; nothing closed.
