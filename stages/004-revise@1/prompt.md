Goal: Run the serialized fabro line: one pass = upstream merge (when the threshold is met, infra-only) OR one develop+revisor cycle; children are created only after the previous child merged

## Context
- child_run_id: 01M1S6VRWQMD56M1X8HWAXDSNN
- journal: {"painpoints":[{"text":"Develop-leg create call: the fabro_run_create tool schema rejects the documented workflow_source-only object form; it only validates when 'workflow' is also present alongside 'workflow_source' (the server prefers the source form). Suggest adding workflow_source to the tool's JSON schema so callers don't burn retries discovering the accepted shape (run 01M1S6MSV4GBEMPMVV6AXWWE8H, 5 failed create attempts)."}],"observations":["Child sat in pending/approval_required for ~2.5 min despite auto_approve=true before a human/auto approval landed at 16:38:58 — matches seed fabro-54f0's zombie-pending concern; the one-child hard rule was honored (no second child created).","Child PR auto-merge did not complete within the 20-minute poll cap (PR #28 open at final check); integration is pending, not failed — the merge leg should pick it up."]}
- merge_status: pending: PR #28 still open at 20-min auto-merge cap; denkhaus tip 7234a7f41 lacks the child tree; run branch fabro/run/01M1S6VRWQMD56M1X8HWAXDSNN pushed at a98db89f — merge leg must re-verify tree equality before the revisor leg
- pr_url: https://github.com/denkhaus/fabro/pull/28
- seed_cycles: {"start":1,"survey":1,"develop":1}
- seed_id: fabro-78ac


You are the Conductor's Revise Leg. You start ONE revisor run against the develop run this pass just integrated, then wait.

## Procedure

1. Read `child_run_id` from context (the develop leg left it). If absent, route "Revisor child failed" with a journal note (pass continuity broken).
2. Create the child: `fabro_run_create` with `{"workflow_source": {"repo": "denkhaus/fabro", "branch": "denkhaus", "workflow": "revisor"}, "environment": "toolchain", "auto_approve": true}`. The revisor itself selects the newest revisable run (ADR-0015) — no goal needed.
3. Wait terminal: poll `fabro_run_get` ~every 60s (up to 30 min). Route "Cycle complete" on any terminal state (the revisor's own soft exits are legitimate outcomes); route "Revisor child failed" only when the run failed hard.

## Workflow addressing (fabro-e297, server-side resolution)

Create child runs with the git workflow source — the server resolves and
registers the workflow versions; the sandbox filesystem never participates:
`{"workflow_source": {"repo": "denkhaus/fabro", "branch": "denkhaus", "workflow": "<name>"}, "environment": "toolchain"}`.
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