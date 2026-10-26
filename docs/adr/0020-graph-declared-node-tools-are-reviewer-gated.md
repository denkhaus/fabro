# ADR-0020: Graph-declared per-node tools are author power, gated by review

- Status: Accepted
- Date: 2026-09-09
- Deciders: user + agent (fabro session, grill over PR #76)
- Related: ADR-0019 (capability gate; this ADR settles where the gate
  lives for one specific surface), fabro-c419 (the scoped approval),
  PR #76 / 0d7ef0d9f (the general mechanism), fabro-154c (reviewer
  ADR-0019 statement)

## Context

fabro-c419 made the engine honor node-level `fabro_tools` attributes
(ADR-0009 stage-envelope family), so a workflow graph can grant a stage's
agent a named subset of the fabro run tools. The recorded user approval
(2026-09-08) covered exactly one binding: read-only `fabro_runs_list` on
the develop planner node. The implementation (PR #76) is deliberately
general: `register_named_fabro_run_tools` accepts ANY tool name from
`fabro_tool::tool_definitions()` — including write-path tools such as
`fabro_run_create` and `fabro_run_interact` — and any node in any
workflow can declare it. Since the line's agents author their own
workflow graphs (`.fabro/workflows/**`) via PRs, this is an
agent-reachable capability surface in the sense of ADR-0019.

ADR-0019 leaves the gate location for this surface open: least privilege
(item 1) suggests an engine-side restriction; items 4-5 suggest the
needs-user + reviewer ceremony. Which one owns the node-level opt-in?

## Decision

The node-level `fabro_tools` mechanism stays GENERAL — no engine-side
read-only allowlist, no per-tool approval list. The gate is the review
chain (user decision 2026-09-09, grill session):

1. **A `fabro_tools` declaration in a workflow graph is a capability
   delta** and is reviewed as one (ADR-0019 items 4-5): the develop
   reviewer states whether the declaration is the minimal set the
   stage's job requires (fabro-154c makes that statement mandatory),
   the closeout surfaces it, and the line-watch treats an unjustified
   escalation as a finding.
2. **The reviewer is trusted to judge minimality.** An engine-side
   allowlist (the rejected alternative) would split the capability
   model into "tools the engine may hand out per node" and "tools only
   the run-wide flag may grant", making the overall logic harder to
   explain and reason about — without removing the trust question, it
   would only relocate it.
3. **The run-wide `run.agent.fabro_tools` flag stays an
   operator-level decision** (settings, not graph): a graph never
   widens attribute-less stages; only the flag does (unchanged PR #76
   semantics).
4. New tool names or NEW tools surfaced through this mechanism remain
   full ADR-0019 item-4 user decisions; this ADR covers WHERE the
   gate for per-node declarations of EXISTING tools lives, not the
   introduction of new capability.

## Consequences

- The general mechanism from PR #76 is ratified as designed; no
  follow-up engine restriction is filed.
- Reviewers and line-watch must know the tool inventory
  (`fabro_tool::tool_definitions()`) to judge escalations; the reviewer
  prompt (fabro-154c) is the enforcement point.
- A merged workflow graph that grants a write tool to a node whose job
  is reading is a review miss under this ADR — revert, not ratify.
- If a real escalation incident occurs, this decision is reopened
  (the rejected alternative is documented above for that day).
