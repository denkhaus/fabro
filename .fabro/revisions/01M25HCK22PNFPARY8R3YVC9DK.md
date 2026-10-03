# Revision — run 01M25HCK22PNFPARY8R3YVC9DK

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M25HCK22PNFPARY8R3YVC9DK.md
- seeds filed: fabro-3e48 (Implementer: add routing-consistency self-check — one route per branch), fabro-d83a (PROJECT_FACTS: name workflow.fabro as the graph source), fabro-b659 (Add reviewer->implementer 'Changes requested (minor)' edge for single mechanical edits), fabro-9bf4 (Bake graphviz into the develop toolchain image — needs-user, capability-affecting per ADR-0019)
- basis: run 01M25HCK22PNFPARY8R3YVC9DK, workflow version 99e5a7758912f1adc63c1749584b00296293249901eafbd60fa4e0556f75e4be, commit add88af03e7532624d73a287ff9d0225a77343a8
- revised_at_commit: add88af03e7532624d73a287ff9d0225a77343a8 (ADR-0015: engine drift signal for later judgement)

## Findings

### Implementer: add routing-consistency self-check — one route per branch
- filed: fabro-3e48 (priority 1)
- Change: in `.fabro/workflows/develop/prompts/implementer.md` (Inline verification report section) require a pre-finish re-read of every routing instruction written — each branch must yield exactly ONE route; two routing sentences in one branch is a FAIL, fix before reporting.
- Expected effect: label-dead-config contradictions caught at implementer time; saves ~one full review cycle (~126 s wall / ~$0.125 / 5 stage visits per occurrence, as measured on this run's pass-2).

### PROJECT_FACTS: name workflow.fabro as the graph source
- filed: fabro-d83a (priority 2)
- Change: add one line to `.fabro/workflows/develop/prompts/project-facts.md`: 'Graph source: `.fabro/workflows/develop/workflow.fabro` (DOT); `workflow.toml` wires runtime settings only — never probe it for edges.'
- Expected effect: ~2 fewer tool calls per fresh claim; planner@1 (100.6 s, $0.117 — costliest stage of this run) burned exactly those two probes on `workflow.toml`. Cross-referenced open fabro-0586 (same file, different content) — complementary, not supersession.

### Add reviewer->implementer 'Changes requested (minor)' edge for single mechanical edits
- filed: fabro-b659 (priority 2)
- Change: add a `reviewer -> implementer` edge labeled 'Changes requested (minor)' (condition `preferred_label="Changes requested (minor)"`) in `.fabro/workflows/develop/workflow.fabro`, with a matching verdict shape in `prompts/reviewer.md` gated on 'feedback is exactly one mechanical edit, fully specified'; everything else keeps the planner hop.
- Expected effect: ~30 s and one LLM call saved per minor cycle; this run's one-sentence-deletion fix took a pointless planner hop (31.5 s / $0.036) whose journal admitted it added nothing; also removes re-plan drift risk.

### Bake graphviz into the develop toolchain image
- filed: fabro-9bf4 (priority 2, labels needs-user,revision — capability-affecting per ADR-0019, implementation awaits explicit user approval)
- Change: add `graphviz` to the apt-get list in `.fabro/Dockerfile.toolchain` (engine-mediated, read-only usage: `dot -Tcanon` syntax verification of graph edits).
- Expected effect: mechanical syntax verification of `workflow.fabro` edits at implementer time; implementer@1's 'eyeballing the edge block' fallback (`dot` not installed) eliminated. Inert until the toolchain image rebuilds (covered by closed fabro-6f6e). Distinct from closed fabro-05d0 (baked `gh`, different package).
