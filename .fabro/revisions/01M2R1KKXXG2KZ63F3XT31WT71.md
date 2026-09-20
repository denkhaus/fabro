# Revision — run 01M2R1KKXXG2KZ63F3XT31WT71

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2R1KKXXG2KZ63F3XT31WT71.md
- seeds filed: fabro-fe6f — Deduplicate ml record filings: one record per lesson, corrections amend
- basis: run 01M2R1KKXXG2KZ63F3XT31WT71, workflow version f4ab9ff48b6e162ee81f42689c29e27253fba4fe3632992c2e164afa31eb5ca5, commit 75799744613fde9aae00d413ea9fcd80f8d67829
- revised_at_commit: 75799744613fde9aae00d413ea9fcd80f8d67829 (ADR-0015: engine drift signal for later judgement)

## Findings

### Deduplicate ml record filings: one record per lesson, corrections amend
- filed: fabro-fe6f
- Change: implementer prompt step 6 (`.fabro/workflows/develop/prompts/implementer.md`) and/or `ml record`: one record per lesson; a correction amends the existing record, never re-files. Evidence: two records for the same lesson filed 7s apart (mx-a74829 full + mx-c83b9d stub) while `lesson_capture` names only mx-a74829.
- Expected effect: the expertise store stops accreting stub duplicates that future `ml search` hits return instead of the real record.
