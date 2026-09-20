# Revision — run 01M1YWHK6F8F2VZ6F6FZV8A7VE

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M1YWHK6F8F2VZ6F6FZV8A7VE.md
- seeds filed: fabro-9a43 — Point reviewer phrase checks at read_file offset/limit when diff line numbers are known
- basis: run 01M1YWHK6F8F2VZ6F6FZV8A7VE, workflow version 5603b5e729f24c8e4ab21d9e7599599d18c4c910b381de3682f7a9a6b8628a21, commit 50367ecee388a4806948c80162dc93c2f95f7bb2
- revised_at_commit: 50367ecee388a4806948c80162dc93c2f95f7bb2 (ADR-0015: engine drift signal for later judgement)

## Findings

### Point reviewer phrase checks at read_file offset/limit when diff line numbers are known — filed fabro-9a43

Concrete change: in `.fabro/workflows/develop/prompts/reviewer.md` (tool-guidance/verification section), instruct the reviewer to verify phrase-presence claims with `read_file` offset/limit on the exact lines the diff names, or pass an explicit max_results to grep, instead of multi-pattern grep alternations.

Expected effect: one fewer verification round-trip per review of text-edit seeds — in the basis run the reviewer's first grep alternation returned truncated output showing only the line-25 match, forcing a second targeted grep (~3–4 s of the reviewer's 23.7 s) for a check the diff had already localized to lines 25 and 78.

Cross-reference: related to open fabro-a0fe (blob reads via `read_file` offset/limit) — different section, different mechanism; not supersession.
