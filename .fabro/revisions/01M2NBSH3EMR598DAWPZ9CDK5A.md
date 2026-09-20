# Revision — run 01M2NBSH3EMR598DAWPZ9CDK5A

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2NBSH3EMR598DAWPZ9CDK5A.md
- seeds filed: fabro-dd4e — Implementer: never chain a script write and its execution in one shell call
- basis: run 01M2NBSH3EMR598DAWPZ9CDK5A, workflow version e1bda2f455a07c0d501ab040c95e4344d05b2f486e5a471db6e4c56668ad1c61, commit cd8ba34905484f2421d1991b7fc1e5d767c8e01f
- revised_at_commit: cd8ba34905484f2421d1991b7fc1e5d767c8e01f (ADR-0015: engine drift signal for later judgement)

## Findings

### Implementer: never chain a script write and its execution in one shell call
- filed: fabro-dd4e
- Change: add one line to the tool-hygiene section of `.fabro/workflows/develop/prompts/implementer.md`: writing a script file and executing that same script must be separate shell calls (write first, then run).
- Evidence: after editing `.fabro/scripts/dup-run-check.nu`, the first chained re-run printed `closure foreign` while an identical immediate re-run printed `closure self` — the first execution raced the heredoc write.
- Expected effect: eliminates flaky PASS/FAIL preflight verdicts on edited scripts — the class behind this run's one unexplained observation.
- New-seed justification: open seeds cover parallel batch edits (fabro-4601), in-batch concurrent same-path writes (fabro-929f, fabro-facd), and null-path exercises (fabro-50f8) — none covers sequential write→execute chaining within a single shell call.
