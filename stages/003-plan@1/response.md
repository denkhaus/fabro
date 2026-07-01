Plan written to `merge-train-plan.md`.

**Summary:**

- **READY (2):** Both PRs qualify — open, not draft, target `main`, head branches in `fabro-sh/fabro` (not forks), both `MERGEABLE`.
  1. **#543** — `feat: grant Dependabot alerts read/write...` (branch `add-dependabot-alerts-app-scope`)
  2. **#544** — `fix: grant organization_projects...` (branch `add-organization-projects-app-scope`)
- **SKIP:** none.
- **Order:** keep requested **543 → 544**. No dependency; independent permission additions.
- **Conflict warning:** #544 **will conflict** after #543 merges. Both edit the identical `"emails": "read"` line and insert a test assertion at the same spot in `install.rs` (both cli + server crates). The conflict is additive/trivial — resolution is to **keep both permission keys** (`vulnerability_alerts` and `organization_projects`) and **both test assertions**, fix trailing commas, then verify with `cargo nextest run -p fabro-cli -p fabro-server`.

Plan path: `/home/daytona/workspace/fabro/merge-train-plan.md`