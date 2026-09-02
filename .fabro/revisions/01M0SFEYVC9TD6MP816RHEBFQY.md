# Revision — run 01M0SFEYVC9TD6MP816RHEBFQY

- status reviewed: failed
- review: .fabro/reviews/develop/01M0SFEYVC9TD6MP816RHEBFQY.md
- seeds filed: fabro-9f97 Skip PR creation when the run diff is journal-only; fabro-696c Fix PR publishing: provision a PR-capable credential or disable PRs; fabro-a54d Make the baked mise toolchain a no-op in run containers

## Findings

### Skip PR creation when the run diff is journal-only
- filed: fabro-9f97
- Change: in `.fabro/project.toml` `[run.pull_request]`, skip PR creation when the diff excluding `.fabro/journal/` is empty. Expected effect: no-op runs report success instead of `failed(publish_failed)`; no squash-merged journal churn into the base branch.

### Fix PR publishing: provision a PR-capable credential or disable PRs
- filed: fabro-696c
- Change: provision the GitHub App credential `project.toml` says PR creation requires, or set `enabled = false` under `[run.pull_request]` until it exists. Expected effect: publish stops being the sole cause of failed runs.

### Make the baked mise toolchain a no-op in run containers
- filed: fabro-a54d (related to fabro-80ec, distinct: env propagation + stale docs)
- Change: fix `MISE_DATA_DIR` propagation in `.fabro/Dockerfile.mise` and `[run.prepare]`, log `mise ls` when install is slow, update stale `.mise.toml` header. Expected effect: ~95s saved per run; setup stops dominating run time.
