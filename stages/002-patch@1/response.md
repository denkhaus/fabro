I've hit a hard blocker and cannot make progress on this task. Here's the precise diagnosis:

## Blocker: no GitHub access + shell is unusable

Every shell invocation — even `echo hello` — fails before executing, with:

```
failed to mint GitHub installation access token:
GitHub App does not have access to repository fabro.
Update the installation's repository permissions to include it.
```

This is fatal for this specific goal, because "triage Dependabot alerts and open verified patch PRs" depends entirely on things I now can't do:

| Step | Requires | Status |
|------|----------|--------|
| List Dependabot alerts | Authenticated GitHub Security API (`security_events: read`) | ❌ no token / no repo access |
| Create branch, commit, push | `git` via shell + repo write | ❌ shell blocked, no access |
| Open PRs | GitHub PR API (`pull_requests: write`) | ❌ no access |
| Verify fixes (`cargo build`, `cargo nextest`) | shell | ❌ shell blocked |

I also confirmed the `eng-patch-cves` skill definition is **not present in the working tree** (`grep` for `patch-cves`, `eng-patch`, `dependabot`, and `cve` finds nothing relevant), even though commit `943dbe224` claims to add it. So I can't recover the exact workflow steps locally either. `read_file` and `grep` work, but `shell` and recursive `glob` are broken in this environment.

## What I did *not* do

I did not fabricate any alerts, branches, or PRs. Nothing was changed.

## What needs to happen to unblock

1. **Grant the GitHub App installation access to the `fabro` repository** (the error message says this directly). The installation needs, at minimum:
   - `security_events: read` (to read Dependabot alerts)
   - `contents: write` + `pull_requests: write` (to push branches and open PRs)
2. **Restore shell execution** — the token-minting failure is happening at the shell-tool wrapper layer and blocks *all* commands, not just GitHub ones. This looks like an infrastructure/credential configuration issue in the run environment rather than something I can fix from inside the workspace.

## How would you like to proceed?

- If you can fix the installation permissions and shell access, re-run `/eng-patch-cves` and I'll do the full triage-and-PR flow.
- Alternatively, if you paste the current Dependabot alert list here, I can prepare the dependency-bump edits locally (e.g., `Cargo.toml` / `bun.lock` changes) so they're ready to push once access is restored — though I still couldn't verify builds without shell.

Which do you prefer?