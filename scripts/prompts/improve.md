# Improve-review prompt

Sent via `fabro ask <run-id>` after a completed run; the answer is saved
under `.fabro/reviews/<run-id>.md` by `scripts/run_workflow.nu`.

## Prompt

Provide recommendations for improving this workflow, including better graph design, prompting strategies, more efficient tool usage, error handling improvements, and ways to optimize the overall user experience.

## Answer shape (guidance, not a hard contract)

- Ground every recommendation in what actually happened in THIS run
  (stage transcripts, gate results, journal observations, timings, cost).
- Order by expected impact; name the file or node to change.
- Keep it actionable: one recommendation, one concrete change, one
  expected effect. No generic best-practice filler.
