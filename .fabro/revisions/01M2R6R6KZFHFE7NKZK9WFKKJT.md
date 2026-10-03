# Revision — run 01M2R6R6KZFHFE7NKZK9WFKKJT

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2R6R6KZFHFE7NKZK9WFKKJT.md
- seeds filed: fabro-d950 — Reviewer: route multi-line/regex nu checks through a heredoc temp file first
- basis: run 01M2R6R6KZFHFE7NKZK9WFKKJT, workflow version f4ab9ff48b6e162ee81f42689c29e27253fba4fe3632992c2e164afa31eb5ca5, commit c6e558042a7c3e0e5d1afbf79883eaafcea9c20e
- revised_at_commit: c6e558042a7c3e0e5d1afbf79883eaafcea9c20e (ADR-0015: engine drift signal for later judgement)

## Findings

- Reviewer heredoc-first for multi-line/regex nu checks — filed fabro-d950. Concrete change: one line in the tools section of `.fabro/workflows/develop/prompts/reviewer.md` teaching heredoc temp `.nu` files as first choice, plus amending the mx-d44874 expertise record via `ml record`. Expected effect: removes ~2 errored shell calls and the retry reasoning per review lap. No duplicates found (fabro-2904 covers prompt-embedded nu -c quoting only; fabro-e6c7 covers implementer hidden-path mode).
