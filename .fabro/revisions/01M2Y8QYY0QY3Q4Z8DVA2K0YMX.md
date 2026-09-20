# Revision — run 01M2Y8QYY0QY3Q4Z8DVA2K0YMX

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2Y8QYY0QY3Q4Z8DVA2K0YMX.md
- seeds filed: none — zero balance credit this pass (no same-pass stale/superseded closes)
- balance: 0 non-exempt seeds filed / 0 — no credit this pass
- basis: run 01M2Y8QYY0QY3Q4Z8DVA2K0YMX, workflow version bdd056c377ba821c9d6cf0cd764c7379961019561cf7e1151acf0b7bec5da4ad, commit 5ff33e8b1460d8e00bc1087e9e1c9309b9e6a9b6
- revised_at_commit: 5ff33e8b1460d8e00bc1087e9e1c9309b9e6a9b6 (ADR-0015: engine drift signal for later judgement)

## Findings

- overflow: Slim the AGENTS.md sd onboarding to a pointer — PROJECT_FACTS already renders the command table — edit AGENTS.md's Seeds section down to a one-line pointer to the workflow's PROJECT_FACTS table (agent.memory.loaded injects all 25,746 bytes of AGENTS.md into every LLM session; planner 7,808 memory tokens, reviewer 6,260 = 41% of its 15K input); effect: ~5–8K tokens saved per LLM stage, every run, all workflows. Dedupe: no existing seed covers memory/prompt duplication (fabro-52b4 splits the PROJECT_FACTS sd table per role, fabro-9588 is engine-level memory scoping, fabro-a701 log levels only). NEXT PASS: re-run dedupe, then file.
- overflow: Fix the PROJECT_FACTS fs_hide bullet: the reviewer node is exempt by design — one-line edit to the shared PROJECT_FACTS fs_hide bullet in `.fabro/workflows/develop/prompts/` stating the hide binds the planner/implementer envelopes and the reviewer node is exempt by design; effect: removes a standing prompt falsehood (Reviewer@1 observed the contradiction) that could force a re-capture cycle on loop-asset seeds. Dedupe: fabro-a512 (workflow.fabro comments), fabro-6997, fabro-b7ab touch different files/mechanisms. NEXT PASS: re-run dedupe, then file.

## Overflow ledger

- The 10 open ledger entries from prior passes remain open (zero credit this pass); no overflow-dup links needed — the two new overflows above are new themes not open in the ledger.
