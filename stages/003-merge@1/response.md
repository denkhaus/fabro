The create tool is returning `Access denied` regardless of payload shape (full spec, and string shorthand), and run lookup confirms the Fabro tool layer is reachable but denying run creation. Per the outcome contract, create-impossible means the pass fails.

```json
{
  "preferred_next_label": "Unrouted merge outcome",
  "outcome": "failed",
  "failure_reason": "fabro_run_create returned 'Access denied' on three attempts (full workflow_source spec with auto_approve, and bare string shorthand); no child run was created and no merge could be started. The Fabro API was reachable (fabro_run_get responded), so this is an authorization/permission failure on run creation, not a schema or connectivity issue.",
  "suggested_next_ids": [],
  "context_updates": {
    "journal": {
      "painpoints": [
        {
          "text": "fabro_run_create returned 'Access denied' (3 attempts, this session, 2026-09-10) with no explanation of which permission is missing: the error carries no field-level or policy detail, only 'Access denied'. Suggest the run-create tool surface the failing permission/policy reason (e.g., missing label, environment not allowed, actor scope) so the conductor can distinguish a transient auth issue from a permanent policy block without burning repeated calls."
        }
      ],
      "observations": [
        "Merge leg aborted before child creation: fabro_run_create access-denied on all payload shapes (workflow_source spec + auto_approve=true, bare string); fabro_run_get works, so API reachable, authorization on create broken. No child_run_id exists, no merge performed. Upstream drift remains at 24 commits (origin/denkhaus..upstream/main, newest 'Merge pull request #848 from fabro-sh/rust-boundary'); next merge leg should retry the child create and, if denial persists, investigate the server-side run-creation permission for the conductor's token/environment 'toolchain'."
      ]
    }
  }
}
```