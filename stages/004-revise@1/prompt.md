Goal: Run the serialized fabro line: one pass = upstream merge (when the threshold is met, infra-only) OR one develop+revisor cycle; children are created only after the previous child merged

## Context
- child_run_id: 01M23EDVNHPGD8QNBWZN8WNTDD
- journal: {"painpoints":[{"text":"Gate stuck on PR #99 (child run 01M23EDVNHPGD8QNBWZN8WNTDD, seed fabro-94e8): fabro_run_wait until=merged returned reached=blocked on all 3 bounded waits (3 × 20 min, ~60 min ceiling) with no terminal check verdict visible from the tool payload — the wait result carries only run status + PR link, not which required check is failing or why. Fix idea: include the failing/blocked check names (or a blocked reason) in the wait tool's blocked return so the conductor can distinguish 'checks still running' from 'required check failed' without a manual GitHub lookup."}],"observations":["child 01M23EDVNHPGD8QNBWZN8WNTDD succeeded: seed fabro-94e8 (auto-rebase for stale run-branch PRs) claimed, implemented (12 files, +784/-3), gate green in-container, review approved first-pass, closeout closed the seed; PR #99 opened but never merged within the 3-wait ceiling","gate wait outcome: blocked × 3 (persistent) — route Gate stuck — revise anyway; merge soft-parked, next fire re-enters via survey; note the new staleness supervisor this very PR adds would retire such a PR after 24h as stale_base once deployed"]}
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
The contract is complete. Do not ask the user to provide or choose the output shape.