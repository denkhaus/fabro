The child creation is impossible: `fabro_run_create` returns "Access denied" on three attempts with the exact prescribed shape. Without a child run id, no wait is possible and the merge leg cannot proceed.

```json
{
  "preferred_next_label": "Unrouted merge outcome",
  "outcome": "failed",
  "failure_reason": "fabro_run_create rejected all attempts with 'Access denied' (3 tries, exact schema: workflow string + workflow_source repo/branch/workflow, environment toolchain, auto_approve). No child run was created, so no terminal or merged wait could be issued. Likely a permissions/environment grant issue for this stage's run-creation scope.",
  "suggested_next_ids": [],
  "context_updates": {
    "journal": {
      "painpoints": [
        {
          "text": "fabro_run_create returns a bare 'Access denied' with no reason, permitted-workflow list, or scope hint (merge leg, run attempts at 2026-09-10, workflow 'merge-upstream' with workflow_source repo denkhaus/fabro branch denkhaus). Fix idea: include the failing authorization scope (stage identity vs workflow slug vs environment 'toolchain') in the denial payload so the leg can distinguish a transient grant gap from a misaddressed workflow source."
        }
      ],
      "observations": [
        "Merge leg pass aborted before child creation: 3x 'Access denied' on fabro_run_create for merge-upstream (workflow_source denkhaus/fabro@denkhaus, environment toolchain). Upstream count from survey context remains 24 vs MIN 5, so the merge is still warranted — the next merge leg should retry creation from a fresh pass and, if denial persists, escalate the stage's run-create grant before touching the /merge-upstream manual skill."
      ]
    },
    "child_run_id": null
  }
}
```