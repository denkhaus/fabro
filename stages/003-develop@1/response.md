All creation channels are exhausted. Summary of this leg:

**Journal**
- Repo root via `pwd`: `/repos/denkhaus/fabro`; workflow file verified present and readable at `/workspace/fabro/.fabro/workflows/develop/workflow.toml` (and under the `/repos/denkhaus/fabro` checkout) in the agent shell.
- `fabro_run_create` attempts (all rejected with `workflow not found`):
  - bare `develop`, `Develop`
  - `.fabro/workflows/develop/workflow.toml` with `cwd` = `/repos/denkhaus/fabro`, `/workspace`, `/workspace/fabro`
  - absolute path `/repos/denkhaus/fabro/.fabro/workflows/develop/workflow.toml`
  - directory form `.fabro/workflows/develop`
  - environment `toolchain`, `mise`, and omitted; with/without `dry_run`
- No fallback exists in this sandbox: no `fabro` CLI binary, no server at `127.0.0.1:32276`, so `just run develop` (`scripts/run_workflow.nu`) cannot create the child either.
- Conclusion: the create tool resolves workflow paths against a filesystem that does not include this worktree — the fabro-e297 interim addressing gap, now failing even with the explicit path + cwd form. **No child run was created; no seed was claimed; nothing merged.**

```json
{
  "outcome": "failed",
  "failure_reason": "Develop child could not be created: fabro_run_create returns 'workflow not found' for .fabro/workflows/develop/workflow.toml under every cwd variant (repo root /repos/denkhaus/fabro, /workspace, /workspace/fabro), bare names, absolute paths, and both environment slugs, even though the file exists and is readable in the agent sandbox; no local fabro CLI/server fallback exists. Create tooling failed (fabro-e297 interim path-resolution gap); no child run, no seed claimed, nothing merged.",
  "preferred_next_label": "Develop child failed",
  "suggested_next_ids": [],
  "context_updates": {
    "journal": "Develop leg (pass after survey 1, drift 1 < 5): create tooling failure. fabro_run_create could not resolve .fabro/workflows/develop/workflow.toml with any cwd/environment combination despite the file existing in the sandbox; no fabro CLI/server fallback available. No child created, no seed claimed. Retriable next fire; file a seed for the create-tool path resolution gap.",
    "seed_cycles": "{\"start\":1,\"survey\":1,\"develop_failed\":1}"
  }
}
```