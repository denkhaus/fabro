# Revision — run 01M20PXPFESQM8HAWF4MSXAKX5

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M20PXPFESQM8HAWF4MSXAKX5.md
- seeds filed: fabro-d02d — Engine: name the shell bypass in the fs_hide read_file denial message
- basis: run 01M20PXPFESQM8HAWF4MSXAKX5, workflow version af036506627c2c235dc58d52e1ceda475fc4053f219b513e9582f644224dc129, commit 570a3f4923032cdcab14bc628652b0212cc981e6
- revised_at_commit: 570a3f4923032cdcab14bc628652b0212cc981e6 (ADR-0015: engine drift signal for later judgement)

## Findings

### Engine: name the shell bypass in the fs_hide read_file denial message — filed fabro-d02d

Planner seq 52–53 of run 01M20PXPFESQM8HAWF4MSXAKX5: read_file on `.fabro/workflows/revisor/prompts/file.md` returned "hidden … behaves as if it did not exist"; the planner burned an LLM round trip reasoning about fs_hide before falling back to sed. Change: append "use a shell command to access this path" to the engine's fs_hide denial text for read_file, so tool feedback teaches the bypass. Expected effect: one saved LLM round trip per platform-path touch, planner and implementer alike. Complements fabro-8296; no duplicate found via sd search.
