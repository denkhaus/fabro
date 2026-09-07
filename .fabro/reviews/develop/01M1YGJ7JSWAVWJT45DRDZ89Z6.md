# Improve review — run 01M1YGJ7JSWAVWJT45DRDZ89Z6

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (7.7 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-07 18:14+0000 by revisor `fabro_ask`

---

All evidence gathered from run events, stage transcripts, the run journal, and worker logs. Grounding numbers first, then the recommendations.

## What this run actually was (from run events + conclusion)

Seed `fabro-1dc9` — a churn-only platform seed (3 conductor files, +27/−3, plus tracker lines). 7m45s wall, **$0.197** total, 387.7k tokens. Stage split: implementer 267s/$0.093 (69% of wall, 47% of cost), planner 64s/$0.067 (34% of cost), reviewer 45s/$0.037, tester 5.4s/$0, evidence 0.5s, closeout 0.3s. Zero retries, zero review cycles — a clean single-pass run, which makes the residual waste below all the more actionable.

## Recommendations, by expected impact

**1. Make the tester gate actually gate this change class — fold `validate-workflows` into `qualitygate`.**
The only real validation of this run's graph/prompt change was an ad-hoc `just validate-workflows` the *implementer* ran (seq 147–155): it first **timed out at 10.3s** (no `timeout_ms` passed), then took **134s on retry — 92% of implementer tool time and ~35% of run wall**. The deterministic tester meanwhile ran green in 5.4s with an 87-byte output: "no crates touched / format clean" — it checked nothing about the change that was shipped. Change: add the validator to the `qualitygate` recipe in the `justfile` (or path-scope the tester for `.fabro/**`-only diffs, seed fabro-01b9), and drop the validate step from implementer.md's guidance. Effect: the gate validates prompt/graph seeds deterministically, and the 10s-timeout-then-134s dance leaves the LLM stage.

**2. Port the "workaround-is-a-painpoint" clause into the implementer journal contract.**
The implementer hit the run's single biggest friction (timeout + 134s cold validate) and journaled `painpoints: []` — the improve loop that scans journals will never see it. This is live evidence for open seed fabro-21c0. Change: `.fabro/workflows/develop/prompts/implementer.md`, Journal section — copy the clause from `reviewer.md` ("a workaround you performed is a painpoint, not an observation"). Effect: cost-bearing friction reaches the improve loop and gets filed as a seed instead of vanishing.

**3. Raise the preamble budget so the evidence capture stops blob-ref'ing.**
The 9.1KB capture was demoted to a blob ref even though the reviewer sat at **1.5% of the 1M window**, forcing a tool round-trip before judging (reviewer response opens "Evidence blob read in full"). The 24KB graph budget was eaten by brief (~2.4KB) + summary + tester section. Change: `.fabro/workflows/develop/workflow.fabro` graph attr `preamble_budget_kb` 24→32 and reviewer `preamble_inline_max_kb` 16→32 (seeds fabro-8d2c/fabro-cf3e). Effect: capture arrives inline; reviewer saves a tool call + LLM turn per review and can't reject on preview-only grounds.

**4. Constrain the conductor's routing label to an enum — the reviewer flagged the exact failure mode this seed was fixing.**
Reviewer journal: the new `develop -> revise` edge "relies on the LLM driver emitting the exact preferred_label string; a typo … would fall through to the existing 'Develop child failed' soft exit." The magic string now appears in 3 places (`develop-leg.md` step 5, its outcome contract, and the edge condition in `workflow.fabro`). Change: in `.fabro/workflows/conductor/workflow.fabro`, give the develop node an `output_schema` enum of its outgoing edge labels (seed fabro-de4d). Effect: a mistyped label becomes a schema validation error instead of silently re-triggering the original bug.

**5. Give the planner a top-N tracker view instead of the 150-issue firehose.**
`sd ready --assignee fabro --limit 200` poured **~20KB / 150 issues (truncated at 19,918 bytes)** into planner context for a decision that needs the top of the list; conversation tokens grew 2.6k→11.7k across the stage, and the planner was 34% of run cost. Change: `.fabro/workflows/develop/prompts/planner.md` sd table — use a bounded view (`--limit 10`) for the claim decision (seed fabro-c3b4). Effect: smaller context per planner turn, fewer dollars on every run's most predictable stage.

**6. Make the implementer's `sd show` re-fetch conditional on brief quality.**
The brief already carried the full seed spec verbatim; the implementer still re-ran `sd show fabro-1dc9` (~1.8KB, one LLM turn) because `implementer.md` step 1 mandates it unconditionally (seed fabro-4881). Change: step 1 → "re-fetch only if the brief is thin or a re-plan". Effect: −1 tool call, −1 LLM turn per implementer pass.

**7. Error handling (engine): retry or quietly defer metadata snapshots; downgrade a known-benign warn.**
From run logs: the init metadata snapshot **failed on DNS (`github.com: Try again`), stalled 5s, and checkpoint metadata stayed degraded for the entire run** (final notice `checkpoint_metadata_degraded`). Change: retry with backoff or write local-first in `fabro_workflow` (event source of the warn). Also `preamble_allow_keys entry absent … key=output.gate_known_bug_hits` (fidelity module) fires on every green first visit by design — downgrade to info. Effect: no 5s init stall on transient DNS, and warn-level logs stop crying wolf.

**8. Stop paying the same 24KB memory three times per run.**
All three agent stages loaded `AGENTS.md` in full (23,967 bytes; ~5.2–5.8k memory tokens each), and `cache_write_tokens=0` across the whole run — no cross-stage prefix sharing (seeds fabro-944d/fabro-9588). Change: per-node memory scoping (planner needs tracker mechanics, not the full build/gate doc). Effect: ~5k fewer input tokens per stage; modest at $0.20/run but scales with every review cycle.

Not inspected: the reviewer's blob-read tool transcript beyond the invocation flags, and PR #45's post-run merge state — both outside what the run events exposed here.
