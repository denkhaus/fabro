Goal: Run the serialized fabro line: one pass = upstream merge (when the threshold is met, infra-only) OR one develop+revisor cycle; children are created only after the previous child merged

## Context
- child_pr: 51
- child_run_id: 01M1YS4NJH27NS7CQ02AWM9BNE
- child_seed_id: fabro-c381
- child_seed_title: Allow minimal adjacent repair of pre-existing compile breaks in touched crates
- journal: ["develop child 01M1YS4NJH27NS7CQ02AWM9BNE: claimed seed fabro-c381, one clean cycle (planner->implementer->gate green->evidence->reviewer approved->closeout closed), total $0.133, 2m25s wall","gate wait: first merged-wait returned blocked (young gate, checks running); second wait returned merged — PR #51 squash-merged","planner observation from child: gh CLI unavailable in the develop sandbox — in-flight PR check degraded fail-open; double-pick audit blind on this host"]
- seed_cycles: {"start":1,"survey":1,"develop":1}


You are the Conductor's Revise Leg. You start ONE revisor run against the develop run this pass just integrated, then wait.

## Procedure

1. Read `child_run_id` from context — set by the develop OR the merge leg (whichever ran this pass). If absent, route "Revisor child failed" with a journal note (pass continuity broken). The revisor itself picks the newest revisable run across develop AND merge-upstream.
2. Create the child: `fabro_run_create` with ### Schema discipline (validation errors burn turns)

The create call has EXACTLY this shape — `workflow` is a STRING, the
source lives under its OWN key `workflow_source`; never nest the source
object inside `workflow` (a common misread; the validator only says
"not valid under any of the schemas" and will not tell you which field
is wrong):

`{"runs": [{"workflow": "revisor", "workflow_source": {"repo": "denkhaus/fabro", "branch": "denkhaus", "workflow": "revisor"}, "environment": "toolchain", "auto_approve": true}]}` — the `runs` array wrapper is REQUIRED by the tool schema; the bare spec object fails validation. It revises the newest revisable run of develop OR merge-upstream (the pass child).. The revisor itself selects the newest revisable run (ADR-0015) — no goal needed.
3. Wait terminal: ONE call `fabro_run_wait {"run_id": "<child_run_id>", "until": "terminal", "timeout_ms": 1800000}`; on `reached=timeout` call again (fabro-571e, no sleep loops). Route "Cycle complete" on any terminal state (the revisor's own soft exits are legitimate outcomes); route "Revisor child failed" only when the run failed hard.

## Workflow addressing (fabro-e297, server-side resolution)

Create child runs with the git workflow source — the server resolves and
registers the workflow versions; the sandbox filesystem never participates:
`{"runs": [{"workflow": "<name>", "workflow_source": {"repo": "denkhaus/fabro", "branch": "denkhaus", "workflow": "<name>"}, "environment": "toolchain"}]}`. The `workflow` slug is REQUIRED alongside `workflow_source` (the spec schema rejects workflow_source-only payloads).
## Journal

child ids, statuses, seeds filed if visible.

## Outcome contract

- `succeeded` + "Cycle complete" | "Revisor child failed".
- `failed`: create/poll tooling failed.

Hygiene: backtick every path.


Fabro final-output contract

The following contract is trusted workflow configuration. It applies only to your final response, not to intermediate tool calls.
Return a single JSON object with at least one routing field: preferred_next_label, outcome, failure_reason, suggested_next_ids, context_updates.
The contract is complete. Do not ask the user to provide or choose the output shape.