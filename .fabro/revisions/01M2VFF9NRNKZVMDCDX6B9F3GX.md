# Revision — run 01M2VFF9NRNKZVMDCDX6B9F3GX

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2VFF9NRNKZVMDCDX6B9F3GX.md
- seeds filed: none — 0 filing credit this pass (ADR-0022), both findings journaled as overflow
- balance: 0 non-exempt seeds filed / 0 — no credit this pass
- basis: run 01M2VFF9NRNKZVMDCDX6B9F3GX, workflow version 58a8171073f77ad9a1b6be42ffc1c46e128578a9b9875aa1a3fd2e4b3aa28c66, commit 5fd0dc07d39b3b4962dcf2aa521948195c4283fe
- revised_at_commit: 5fd0dc07d39b3b4962dcf2aa521948195c4283fe (ADR-0015: engine drift signal for later judgement)

## Findings

### Phrase planner drift criteria behavior-scoped and sandbox-decidable — overflow to journal
No seed filed: 0 filing credit (no same-pass stale/superseded closes). Concrete change: in `.fabro/workflows/develop/prompts/planner.md` step 7 (contradiction check), require drift criteria phrased behavior-scoped ('rendering-behavior change', not 'commits touching <paths>') and require every check bullet to be decidable with the sandbox's tool set, annotating the chosen reading when the seed text is looser. Expected effect: ~3 fewer reviewer LLM rounds per gate-record seed and no false 'Changes requested' on refactor churn (this run: events seq 94-100 multi-turn `git log --name-only` adjudication of behavior-neutral type migrations; 'PRs unmerged/unanswered' bullet half-undecidable since gh is forbidden). Distinct from open `fabro-b8ed` (per-criterion checks) and `fabro-dad8` (probe-contradiction exemptions).

### Stop the by-design context_update_dropped warn on engine-written output.planner — overflow to journal
No seed filed: 0 filing credit. Concrete change: add `output.planner` to the planner's `context_allow_keys` in `.fabro/workflows/develop/workflow.fabro`, or downgrade the engine's drop-warn to info for engine-generated `output.*` keys. Expected effect: warn-level run logs stop crying wolf per green pass (event seq 61 of this run). Distinct from open `fabro-8275` (read-side preamble allow-key warn) and closed `fabro-169b` (different key, `lesson_capture` on implementer node).
