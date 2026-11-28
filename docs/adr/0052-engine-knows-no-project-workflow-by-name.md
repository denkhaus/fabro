# ADR-0052: The engine knows no project workflow by name

- Status: Accepted
- Date: 2026-11-28
- Deciders: user + agent (iterate session)
- Related: ADR-0017 (engine stays tool-agnostic), ADR-0012 (dogfooding), ADR-0039 (internal integration)
- Supersedes: the ADR-0039 stage-D "hardcoded carrier" decision and every
  engine-side dispatch on a workflow slug that followed it (denkhaus/work#1436,
  work#1488, work#1704)

## Context

Fabro is a multi-project tool. Every project defines its own workflows in any
constellation it needs. The fabro project develops itself with its own Fabro
line; that is dogfooding (ADR-0012), a special case and never a license.

The special case leaked into the engine. `fabro-server` held a fixed table
naming the fabro project's workflows (`merger`, `adjudicator`, `triager`,
`develop`) and dispatched on the slug: a scheduled fire of a named workflow
was skipped when a server-side worklist was empty, and only the run of a named
workflow could write a verdict field, close work items, write triage decisions,
or use the merge door and the resolve push door. ADR-0039 stage D recorded the
hardcoding as a deliberate decision for the fabro line; three later work items
built on it. Tests pinned the coupling. No check enforced ADR-0017.

The cost surfaced on 2026-11-28: the merger gear was enabled and did not run
for 18 slots, because no pull request was open, and the status could not tell
that skip from a dead scheduler (denkhaus/work#1716).

## Decision

1. **The engine never references a project's workflow by name.** No code
   under `lib/` or `apps/` names a workflow of the fabro project or of any
   other project. The engine offers capabilities; a project says which of its
   workflows holds them.
2. **Everything a workflow needs from the engine is a declaration** in the
   workflow definition (`workflow.toml`) or in the project definition
   (`project.toml`). There is no engine-side allowlist, table, or constant
   keyed by a workflow name.
3. **The fire condition is declared** by the workflow (`[schedule.when_empty]`,
   generic worklist kinds). A workflow without the declaration fires at every
   slot. The scheduler never decides that a workflow is idle.
4. **Permissions are declared** by the workflow (`[run.grants]`) and carried
   by its run. A door checks the grant, never the slug.
5. **The rule is enforced mechanically.** A lint turns the gate red when a
   project workflow name appears as a literal in engine code. Reviews treat
   such a literal as a finding, not a judgement call.

## Consequences

- A project can define, rename, and combine workflows without an engine change.
- Engine work for the fabro line is always a capability plus a declaration in
  the fabro project, never a special case.
- A grill that arrives at "the engine knows workflow X" has taken a wrong
  turn and stops there.
- Removing the existing coupling is tracked in denkhaus/work#1716; until it
  lands, every remaining slug literal in engine code is named debt.
