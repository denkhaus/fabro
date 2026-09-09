# Revision — run 01M22PCGN4E3X1XGN630MDDH39

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M22PCGN4E3X1XGN630MDDH39.md
- seeds filed: fabro-a18d — Planner: observations naming consistency work must become current_seed_brief bullets; fabro-7b2a — Implementer prompt: cap step 4 operative rule, move run citations to footnotes; fabro-b94d — Develop: stop ml record from mutating .mulch/mulch.config.yaml mid-run
- basis: run 01M22PCGN4E3X1XGN630MDDH39, workflow version e6202c3d2f79d43966a4f1fc60620f8b3f3d8c36f6facf82a41cd18b0c9c1b4e, commit 6d05194d18b7f1085cbac66a27b5cee0defacce9
- revised_at_commit: 6d05194d18b7f1085cbac66a27b5cee0defacce9 (ADR-0015: engine drift signal for later judgement)

## Findings

### Planner: observations naming consistency work must become current_seed_brief bullets
- filed: fabro-a18d
- In `.fabro/workflows/develop/prompts/planner.md` steps 6/7, require that any journal observation naming required consistency or scope work be folded into `current_seed_brief` as an explicit bullet (or explicitly waived there). Effect: prevents self-contradictory prompt text surviving the loop; zero-cost correctness win.

### Implementer prompt: cap step 4 operative rule, move run citations to footnotes
- filed: fabro-7b2a
- In `.fabro/workflows/develop/prompts/implementer.md`, keep the step-4 operative rule to 3–4 sentences, move the four historical run citations to footnotes or the `ml` expertise store, and add a no-recursion line for platform-targeting seeds. Effect: smaller, faster implementer prompts every run; removes the self-reference confusion round.

### Develop: stop ml record from mutating .mulch/mulch.config.yaml mid-run
- filed: fabro-b94d
- Pre-create the `ml` workflow domain so `ml record` stops auto-creating it and writing tracked-config churn mid-run; until then classify that file as expected churn in `evidence.nu`. Effect: no surprise tracked-config churn per lesson capture. Orthogonal to fabro-96bd; sibling of fabro-6db3.
