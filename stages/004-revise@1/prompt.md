Goal: Run the serialized fabro line: one pass = upstream merge (when the threshold is met, infra-only) OR one develop+revisor cycle; children are created only after the previous child merged

## Context
- child_run_id: 01M1YY8JJCE31XHSDCZ816634G
- journal: ["upstream count: 0 commits ahead of `origin/denkhaus` (`git rev-list --count origin/denkhaus..upstream/main` = 0); newest upstream subject: 'Bump version to 0.348.0-nightly.0'; well below the MIN threshold of 5, so no merge slot is consumed this pass — drift accumulates toward the threshold","revisor backlog: 01M1XCHJTDBJKZ31VFMY1KD61Q, 01M1YBBXR99PNME6Y8ZED4Y2NN, 01M1Y9N7R47JPR49CYXP8BQYC3, 01M1Y852BN91NSVSHFWNWTW5ST — completed develop runs with no revisor run sharing their parent (pairing checked via parent_id across both workflows); the revisor burns these newest-first one per pass","caveat (best-effort): 01M1XCHJTDBJKZ31VFMY1KD61Q (07:32) predates the run at 12:01 that WAS revised; per ADR-0015 stale-evidence runs may be deliberately skipped rather than backlogged, so the older entries may never be revisable — the signal is unambiguous for 01M1Y852BN91NSVSHFWNWTW5ST through 01M1YBBXR99PNME6Y8ZED4Y2NN, ambiguous for the 07:32 run","nothing hurt: both fetches clean, remote `upstream` already configured -> `https://github.com/fabro-sh/fabro`"]
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