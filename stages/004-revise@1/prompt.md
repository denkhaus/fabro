Goal: Run the serialized fabro line: one pass = upstream merge (when the threshold is met, infra-only) OR one develop+revisor cycle; children are created only after the previous child merged

## Context
- child_run_id: 01M292T3YZ3B3YXXPT1R7F82XK
- journal: {"painpoints":[],"observations":["develop child 01M292T3YZ3B3YXXPT1R7F82XK succeeded as an 'Already landed' exit: fabro-7893 closed via superseded-close (fix already in base history via PRs #126/#127), journal+tracker-only diff, PR #128 merged on first wait (gate outcome merged). Next develop leg: top ready candidate is fabro-3ef7 (SQLite database-is-locked + projection self-heal); fabro-5082 (P0) remains in_progress, fabro-6beb needs-user. Revisor should review the child's journal/tracker-only change."]}
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
## Journal — every pass

Report through `context_updates.journal` on EVERY pass. Silence is a missing report, not an empty one. Always emit BOTH keys:

{"journal": {"painpoints": [{"text": "<what hurt and a concrete suggestion, self-contained: where (file/line), what happened, evidence (run id), fix idea>"}], "observations": ["<child run ids, child statuses, seeds filed if visible — what the next revise leg should know>"]}}

- `painpoints`: friction in the orchestration loop itself (tool schema misses, wait semantics surprises).
  Do not fix platform assets — report them here. `[]` when nothing hurt.
- `observations`: at least one entry. The literal `"none"` is a valid
  answer when the pass was genuinely unremarkable — but the key must be
  present every time.
The engine records it durably per stage (no restating, no rewriting);
nobody re-reads your prose, only the JSON survives.

## Outcome contract

- `succeeded` + "Cycle complete" | "Revisor child failed".
- `failed`: create/poll tooling failed.

Hygiene: backtick every path.


Fabro final-output contract

The following contract is trusted workflow configuration. It applies only to your final response, not to intermediate tool calls.
Return a single JSON object with at least one routing field: preferred_next_label, outcome, failure_reason, suggested_next_ids, context_updates.
preferred_next_label must be one of this node's outgoing edge labels: "Cycle complete", "Revisor child failed", "Unrouted revise outcome".
The contract is complete. Do not ask the user to provide or choose the output shape.