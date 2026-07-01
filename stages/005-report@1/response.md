# Merge-Train Run Summary

**Run ID:** 01KWFTMDQVT7SD0GRRWKXH776E
**Order processed:** 543 → 544 (as requested)
**Method:** squash merge, each rebased onto cumulative `main`
**Final base SHA:** `ec0a08afb3325a20b1d9cbf5b5b75b987202e546`

## Per-PR Results

### PR #543 — `feat: grant Dependabot alerts read/write` (branch `add-dependabot-alerts-app-scope`)
- **Status: MERGED**
- **Method:** squash
- **Resulting base SHA:** `3e0db1feb...` (merge commit)
- Rebased onto latest `main`, verified green locally and in CI, merged clean with no conflicts.

### PR #544 — `fix: grant organization_projects app scope` (branch `add-organization-projects-app-scope`)
- **Status: MERGED**
- **Method:** squash
- **Resulting base SHA:** `ec0a08afb3325a20b1d9cbf5b5b75b987202e546` (final main)
- Rebased onto `main` after #543 landed. Encountered the **anticipated additive conflict** in `install.rs` (both cli and server crates) — resolved by keeping **both** permission keys (`vulnerability_alerts` + `organization_projects`) and **both** test assertions, then verified with tests. Merged cleanly; no branch-protection rejections.

## Blocked / Skipped
None. Both queued PRs merged successfully.

## Next Actions
- **#543:** None — merged.
- **#544:** None — merged.

**Train outcome:** Complete. All 2 queued PRs squash-merged in order; no PR was skipped or blocked; final `main` is at `ec0a08afb3325a20b1d9cbf5b5b75b987202e546`.