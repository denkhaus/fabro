# Revision — run 01M2KC8BDP4AJZ5NR0CY2KF5JZ

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2KC8BDP4AJZ5NR0CY2KF5JZ.md
- seeds filed: fabro-58cb — Develop PROJECT_FACTS: name the merge-target branch (`origin/denkhaus`) and use it in implementer duplicate preflight
- basis: run 01M2KC8BDP4AJZ5NR0CY2KF5JZ, workflow version 0a429145a094e46a9426d9a68bbad4a08facc0ea114b9e4840307bc7d7e1265b, commit 2df7fc0f37deb0c997d434975034e13f192411ca
- revised_at_commit: 2df7fc0f37deb0c997d434975034e13f192411ca (ADR-0015: engine drift signal for later judgement)

## Findings

### Implementer duplicate preflight greps the wrong branch (filed: fabro-58cb)

The just-landed duplicate-run preflight (fabro-5656) in `.fabro/workflows/develop/prompts/implementer.md` greps `origin/main`, but `origin/main` is the upstream fabro-sh mirror; the loop integrates on `origin/denkhaus`. The guard can never match a sibling duplicate. Fix: add a merge-target branch fact to `.fabro/workflows/develop/prompts/project-facts.md` and reference it from the preflight; also removes the planner's per-claim `git remote show origin` probe. Complements fabro-1128 (revisor prompt fix), not a duplicate.
