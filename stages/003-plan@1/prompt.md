Goal: Merge the pull requests 543,544 into main as a merge train: process them in the given order, rebasing each onto the latest main (so each PR is validated against the cumulative result of the ones before it), resolving conflicts, pushing, and merging with the squash method. Stop at the first PR that cannot be merged cleanly and report why.

## Completed stages
- **preflight**: succeeded
  - Script: `gh auth status 2>&1 && git fetch --prune origin 2>&1 && echo 'preflight ok'`
  - Output:
    ```
    github.com
      ✓ Logged in to github.com account fabro-sh-fabro[bot] (GITHUB_TOKEN)
      - Active account: true
      - Git operations protocol: https
      - Token: ghs_************************************
    preflight ok
    ```


You are planning a **merge train**. The ordered, comma-separated list of PR
numbers to merge is in the goal (input `prs`), targeting the base branch (input
`base`). You will NOT merge, push, rebase, or change any code in this stage —
this is analysis only. Produce a plan for the next stage to execute.

## Steps

1. Parse the PR list from the goal, preserving the given order.
2. For each PR, gather facts with the `gh` CLI, e.g.:
   `gh pr view <n> --json number,title,state,isDraft,mergeable,baseRefName,headRefName,headRepositoryOwner,author,url`
3. Classify each PR into exactly one bucket:
   - **READY** — open, not draft, targets the base branch, and its head branch is
     in THIS repository (not a fork). Only READY PRs are eligible to be merged.
   - **SKIP** — closed/already merged, draft, targets a different base, or its head
     is on a **fork** (we cannot push conflict fixes to a fork's branch, so the
     train can't rebase it). Record the specific reason.
4. Decide the final merge order. Default to the given order. If you can tell from
   the diffs that one PR logically depends on another, note it, but do not
   silently reorder — call out any ordering concern for a human to confirm.
5. Flag likely conflicts: if two READY PRs touch the same files/regions, note that
   the later one will almost certainly need conflict resolution once the earlier
   one merges (this is expected and the train will handle it).

## Output

Write `merge-train-plan.md` containing:
- The ordered list of READY PRs (number, title, head branch) to merge.
- The SKIP list with a one-line reason each.
- Any ordering or conflict warnings.

Then respond with a short summary and the path to the plan file. If there are
zero READY PRs, say so plainly — the train stage will simply report nothing to do.
