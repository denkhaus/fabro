# Workflow-Revisor: Ask-Fabro as a stage tool, HITL-gated meta-improvement

Decided in the 2026-08-27 grill-with-docs session (revisor design tree;
0010 stays reserved for the exit-kinds decision it already references,
fabro-b907).

## Context

Improving the develop workflow from run evidence is today a HOST-side
step: `run_workflow.nu` calls `fabro ask` (the Ask-Fabro function:
a run session whose analyst agent holds `fabro_run_events` and
`fabro_run_get` for the run id passed on the CLI) after every run and
commits the answer to `.fabro/reviews/<workflow>/<run-id>.md` by hand.
Everything the analyst needs is already server-side; nothing of it is
reachable from inside a stage — the product sandbox is clone-based and
holds no fabro CLI, no server credentials (fabro-d810's lesson).

The code already contains the missing bridge, dead: the workflow stage
handler can register the run tools (`register_fabro_run_tools`), but no
production site ever sets `fabro_run_tools: Some(..)`. The Ask-Fabro
path proves the pattern end to end: worker token scoped to the passed
run id, `ClientBackend::with_run_scope`, read-only auto-approved tools.

## Decisions

- **`fabro_ask` stage tool.** A revisor stage may call
  `fabro_ask { run_id, question }`: the engine creates an Ask-Fabro
  run session on the TARGET run id and returns the analyst's answer to
  the stage. The analyst keeps its own fixed toolset — `fabro_run_events`,
  `fabro_run_get`, plus the new `fabro_run_logs` — always scoped to the
  target run, always read-only.
- **`fabro_run_logs` tool.** Read-only access to one run's persisted
  worker tracing log (WARN/ERROR digest included). This replaces any
  plan to materialize logs as files or turn warnings into events: the
  events-vs-tracing separation (events-strategy.md) stays untouched.
- **`fabro_runs_list` + revises scope.** The revisor does its own
  bookkeeping: enumerate runs of the workflows its graph declares
  (`revises = "develop"` node/graph attribute defines both permission
  and domain). The token scope allows session creation and reads only
  for runs of the declared workflows.
- **Artifacts ride the revisor's OWN run branch** — never the target
  run's branch (usually already auto-merged): the Ask-Fabro answer as
  `.fabro/reviews/<workflow>/<run-id>.md` (same shape as the manual
  file), the revisor's own report as `.fabro/revisions/<run-id>.md`
  (this file doubles as the bookkeeping marker: no report on the base
  branch = run not yet revised), and the resulting seeds via `sd create`.
- **The revisor never changes code.** Write scope of the revisor node is
  restricted by tool policy (47b5 family) to those three destinations;
  the prompt contract states analysis-only. Implementation of filed
  seeds belongs to the develop workflow.
- **Defer-while-active.** The bookkeeping stage skips the commit phase
  while a target-workflow run is active or its PR is open (`.seeds/
  issues.jsonl` is the only shared mutable file); the next cron tick
  retries. If a PR stalls on a conflict anyway, bookkeeping self-heals:
  an unmerged report means the run counts as unrevised next time.
- **Auto-merge from day one; the HITL interview gate is the control.**
  The revisor uses native question tools to grill the human where
  evidence is ambiguous (first production use of the interview
  functions); nothing merges without that approval. No draft phase.
- **Cron via native `fabro-automation` ScheduleTriggers** once three
  manual starts have produced useful seeds (quality ramp before
  automation).

## Consequences

- Engine work is additive: two new read-only tools, one new tool for
  stages, one scope class, one policy attribute — no change to the
  events strategy, no per-run files in git beyond the artifacts.
- The manual improve step in `run_workflow.nu` remains the fallback
  until the revisor is proven; it is retired when cron is enabled.
- The d810 blocker class (unactionable seeds in the product tracker)
  dissolves under dogfooding (ADR-0012); until then the revisor files
  seeds only where they are actionable.

## Amendment (2026-08-31): the graph attribute is `inspects`, not `revises`

During implementation review the attribute name was reconsidered: the
grant is a *capability* (read-only enumeration and Ask-Fabro authority
over the declared workflows' runs), and the revisor is only its first
consumer. Naming the mechanism after one motive (`revises`) would
mislabel any future consumer — audit, triage, digest workflows. The
attribute, worker-token claim, and scope vocabulary therefore use
`inspects` ("this workflow inspects these workflows"). The revisor
remains the ADR's subject; the mechanism it builds on is the neutral
inspection scope. Decided jointly with the user in the 2026-08-31
session; the seeds use the new name from fabro-0c32 onward.

## Amendment (2026-09-02): the inspection scope also gates the legacy run-tool routes (fabro-4556)

`require_run_management_target` historically let any `agent:run_tools`
worker hit per-run routes (events, state, questions, start/cancel/interrupt/
steer, parent-link, pair) for ARBITRARY run ids — a documented bypass of
the `inspects` gate, which then protected only ask/session creation and
enumeration. Decided with the user (option a of fabro-4556): tighten before
the revisor exists, while no production consumer relies on the bypass.

A run-tools worker may now target a run only when it is:

1. the worker's own run, or
2. a run of a workflow its token declares in `inspects`, or
3. a run the worker **created** (engine-stamped `created_by` provenance), or
4. a run **descended from** the worker's run (parent-id ancestry walk,
   cycle-guarded; the walk fails closed beyond 31 hops).

Allowances 3 and 4 keep the sub-workflow flow intact (parent worker
creates a child, links itself, reads child state) and close the
false-parenthood escalation: provenance is engine-stamped at creation, so
a worker cannot mint read access by linking itself to a foreign run —
that link is itself denied by the same rule. Denials log a `worker_auth`
warning and return 403; unknown targets return 404 (user-path parity).
The boundary is pinned by
`run_tool_worker_cross_run_routes_require_inspects_scope` plus the pair
boundary tests; `require_run_management_actor` (no-target routes such as
create) is unchanged.
