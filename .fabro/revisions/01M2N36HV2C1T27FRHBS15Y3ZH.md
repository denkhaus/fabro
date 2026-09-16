# Revision — run 01M2N36HV2C1T27FRHBS15Y3ZH

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2N36HV2C1T27FRHBS15Y3ZH.md
- seeds filed:
  - fabro-b95d — Prompt-lint: flag double-quoted nu -c snippets containing $ in workflow prompt files
  - fabro-af97 — Bake a validate-capable fabro CLI into the run toolchain image (capability-affecting, needs-user per ADR-0019)
- basis: run 01M2N36HV2C1T27FRHBS15Y3ZH, workflow version 6ad783be021e539bec3ef20d5ab0773111d20471eab6b0892bc38e48239d1d19, commit 81b65d1483d5c594cc0ebb17869385b84198e3a8
- revised_at_commit: 81b65d1483d5c594cc0ebb17869385b84198e3a8 (ADR-0015: engine drift signal for later judgement)

## Findings

### Prompt-lint: double-quoted nu -c snippets with $ in workflow prompts
Filed as fabro-b95d. Concrete change: extend `.fabro/scripts/prompt-lint.nu` to extract fenced shell commands from `.fabro/workflows/**/prompts/*.md` and flag double-quoted `nu -c` blocks containing `$`. Expected effect: fewer failed-call/retry inference turns (run evidence: 9/56 shell calls errored; 621.8 s inference vs 6.4 s tool time; $0.764 = 84% of run cost). Not a duplicate: fabro-f18a lints JSON tool-call examples only; fabro-b6f9/fabro-8d4c govern planner probes; closed fabro-88f2 covered nushell-script linting.

### Validate-capable fabro CLI in the run toolchain image
Filed as fabro-af97 (labels: needs-user,revision). Concrete change: include a validation-scoped fabro CLI in `.fabro/Dockerfile.toolchain` so workflow-authoring seeds can run `fabro validate <workflow>` instead of hand-rolled parse-level reasoning. Capability-affecting per ADR-0019 (adds a binary to an agent-reachable surface) — implementation awaits explicit user approval; engine-mediated/read-only directions only. Cross-references open user-owned fabro-fe15 (debug fabro-cli variant of the same target); supersession candidate (user-owned, not closed) — human gate decides.
