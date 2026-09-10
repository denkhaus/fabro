The `fabro_run_create` call is being rejected with "Access denied" on both attempts — the create itself is impossible, so no child run exists and no wait can begin. Per the outcome contract, that's a `failed` pass (create impossible), not a merge-child failure.

```json
{
  "outcome": "failed",
  "failure_reason": "fabro_run_create for merge-upstream (workflow_source {repo: denkhaus/fabro, branch: denkhaus, workflow: merge-upstream}, environment=toolchain, auto_approve=true) returned 'Access denied' on two consecutive attempts; no child run could be created, so the merge leg could not start. Likely an authorization/permission issue on the runs-create API for this session.",
  "preferred_next_label": "Unrouted merge outcome",
  "suggested_next_ids": [],
  "context_updates": {
    "journal": {
      "painpoints": [
        {
          "text": "fabro_run_create returned bare 'Access denied' (run 01M267M2EF8GPWQF32MT9RZGTB, merge leg) with no error code or scope hint; retried once with an identical payload, same result. Fix idea: include the missing permission/scope and the target workflow slug in the create error so the conductor can distinguish token expiry from policy denial without burning turns."
        }
      ],
      "observations": [
        "merge leg aborted before child creation: run-create denied twice at 2026-09-10; no child_run_id exists, no merge pass consumed. Upstream merge is still pending (count 24 >= 5 threshold, newest 'Merge pull request #848 from fabro-sh/rust-boundary'). Next merge leg should retry the same create shape after the access issue is resolved."
      ]
    }
  }
}
```