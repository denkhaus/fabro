Goal: Run the serialized fabro line: one pass = upstream merge (when the threshold is met, infra-only) OR one develop+revisor cycle; children are created only after the previous child merged

## Context
- journal: Upstream drift count: 0 (origin/denkhaus..upstream/main), below MIN threshold 5 -> route Work. No newest upstream subject applicable (zero commits ahead). Remote `upstream` -> `https://github.com/fabro-sh/fabro` verified; fetched upstream + origin with --prune. Nothing hurt besides noisy fetch output (many new remote-tracking branches). seed_cycles start=1.
- seed_cycles: {"start":1,"survey":1}


You are the Conductor's Develop Leg. You start ONE develop run and wait for its integration. You never implement yourself.

## Procedure

1. Create the child: `fabro_run_create` with ### Schema discipline (validation errors burn turns)

The create call has EXACTLY this shape — `workflow` is a STRING, the
source lives under its OWN key `workflow_source`; never nest the source
object inside `workflow` (a common misread; the validator only says
"not valid under any of the schemas" and will not tell you which field
is wrong):

`{"runs": [{"workflow": "develop", "workflow_source": {"repo": "denkhaus/fabro", "branch": "denkhaus", "workflow": "develop"}, "environment": "toolchain", "auto_approve": true}]}` — the `runs` array wrapper is REQUIRED by the tool schema; the bare spec object fails validation (NO goal: the planner picks the most relevant open seed; goalless = autonomous queue burn-down). Record `child_run_id`.
2. Wait terminal: ONE call `fabro_run_wait {"run_id": "<child_run_id>", "until": "terminal", "timeout_ms": 3600000}` — it blocks until terminal or the 60 min deadline. `reached=timeout` (child still running): call again. Never shell-sleep poll loops (fabro-571e).
3. Child FAILED: route "Develop child failed" + journal the reason (retriable causes simply end this pass; the next fire retries the seed).
4. Child SUCCEEDED with goal "Tracker empty"-like completion and no PR: route "Tracker empty" (journal it — the queue is done; the human seeds new demand).
5. Child SUCCEEDED: wait for PR auto-merge: `fabro_run_wait {"run_id": "<child_run_id>", "until": "merged", "timeout_ms": 1200000}` — `reached=merged` -> route "Develop integrated" (context key `child_run_id` stays for the revisor leg); `reached=timeout` -> call again; `reached=closed_unmerged` or terminal-failed -> journal it and route "Develop child failed".

## Workflow addressing (fabro-e297, server-side resolution)

Create child runs with the git workflow source — the server resolves and
registers the workflow versions; the sandbox filesystem never participates:
`{"runs": [{"workflow": "<name>", "workflow_source": {"repo": "denkhaus/fabro", "branch": "denkhaus", "workflow": "<name>"}, "environment": "toolchain"}]}`. The `workflow` slug is REQUIRED alongside `workflow_source` (the spec schema rejects workflow_source-only payloads).
## Hard rule — exactly ONE child

Create AT MOST ONE develop child per pass. If the create call returns an
error, or the created child sits pending (e.g. approval_required), NEVER
create a second child — poll the EXISTING one (fabro_run_get by the id
you recorded) or route "Develop child failed". A start/approval failure
is a state to observe, not a signal to recreate (first production pass:
the agent recreated a child after an approval_required start error and
produced two parallel develops — a serialization violation cleaned up
manually, 2026-09-05).

## Journal

child id, seed title if visible, status, gate wait.

## Outcome contract

- `succeeded` + "Develop integrated" | "Tracker empty" | "Develop child failed".
- `failed`: create/poll tooling failed.

Hygiene: backtick every path.


Fabro final-output contract

The following contract is trusted workflow configuration. It applies only to your final response, not to intermediate tool calls.
Return a single JSON object with at least one routing field: preferred_next_label, outcome, failure_reason, suggested_next_ids, context_updates.
The contract is complete. Do not ask the user to provide or choose the output shape.