# Improve review — run 01M2MRWD9B8CE31MMSG5MZ7MR2

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (3.9 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-16 09:39+0000 by revisor `fabro_ask`

---

All recommendations below are grounded in this run's events, worker logs, and tracker (`.seeds/issues.jsonl`, verified open). Every one maps to an existing seed — no new seeds needed. Run baseline for impact math: 3m52s wall, $0.227 LLM cost; planner 71s/$0.088 (39% of cost), implementer 107s/$0.106 (47%), reviewer 19.5s/$0.033; scripts (tester/evidence/closeout) 4.2s total.

## Ordered by expected impact

**1. Shrink the planner's candidate listing (prompting/tool usage) — seed fabro-66bc**
What happened: the planner's first call `sd ready --assignee fabro --limit 200` poured 200 seeds / 27,804 bytes (stdout truncated) into context (run events seq 30), and it then picked the first High line anyway; planner turn-1 input was 14,965 tokens, total 30.5k input tokens for a claim-and-brief job.
Change: `prompts/planner.md` command table — replace the raw `--limit 200` listing with a top-N priority-sorted `sd ready` invocation.
Effect: ~25 KB less planner context every run; cheaper/faster candidate selection in the stage that is 39% of run cost.

**2. Cap the implementer step-4 citation wall (prompting) — seed fabro-7b2a**
What happened: the implementer was 47% of run cost ($0.106, 104s inference vs 2.5s tool time) for a 6-line Markdown edit, while re-reading a step-4 paragraph stuffed with four Rust-gate run citations (visible in the stage prompt, ~2.3k chars) irrelevant to a config-only seed.
Change: `prompts/implementer.md` step 4 — 3–4 sentence operative rule, move the fabro-0d56 / 01M1YJ8R / 01M11P68SHF / 01M20T9S8 citations to footnotes or the ml store (mx-e26272), and resolve the mechanical-rewrite clause contradiction the seed already flags.
Effect: smaller implementer prompt on every pass of the costliest stage; removes the known self-reference confusion round.

**3. Make evidence survive the renderer cap (error handling) — seed fabro-meta-c9f2**
What happened: the reviewer's preamble rendered the evidence section with "(18 lines omitted)" (reviewer stage prompt), so the reviewer re-verified the edited section via its own shell call instead of judging from the capture — cheap here, but this exact truncation class produced two-cycle rejections on bigger diffs per the seed's basis.
Change: `.fabro/workflows/develop/scripts/evidence.nu` — budget the capture to the *rendered* window (compact integrity header + diff first), per the seed's fix plan.
Effect: reviewer approves from visible evidence in one cycle; shrinks the Verification-blocked failure class.

**4. Give the evidence node its own output key (graph design) — seed fabro-9ef9**
What happened: `command.output` was written by tester (checkpoint seq 177), silently overwritten by evidence (seq 185), then by closeout (seq 219) — the last-writer-wins clobber recurred live; it only didn't bite because the gate was green, exactly the seed's basis.
Change: `workflow.fabro` evidence node — key its output to `evidence.output` instead of the shared `command.output`.
Effect: on a red-gate bounce the gate log stays readable in context, which is precisely what the gatebounce node consumes.

**5. Document the ml record flag contract (prompting/error handling) — seed fabro-96bd**
What happened: the implementer's first `ml record` failed ("pattern records are missing required flag(s): --name", journal observation; 12 shell calls, 1 error), because the taught form omits `--name`; the failure rode stdout with exit 0.
Change: `prompts/implementer.md` step 6 — document the exact flag set per record type (pattern → `--name`; decision → `--title/--rationale`, per the seed's fresh-evidence note) and fix the same wrong example in `AGENTS.md`.
Effect: one fewer failed tool call + LLM round per lesson capture, on every run that records a lesson.

**6. Carry tool errors through the provider protocol (error handling) — seed fabro-b09c**
What happened: worker logs show 6 warnings "this provider protocol does not support the tool result error flag" (09:33:40–09:34:12), clustered exactly around the failed ml-record turn — errors are text-sniffed from stdout, which is why #5's failure looked like success.
Change: provider protocol layer — propagate the tool-result error flag instead of stdout sniffing.
Effect: deterministic error detection for agent tool calls; the retry-hint path stops depending on the model noticing prose.

**7. Make PR-body generation non-strict (error handling/UX) — seed fabro-41b1**
What happened: worker log 09:34:56 — "PR content structured generation failed; retrying once without strict JSON output … the model did not return a JSON document" while composing PR #169.
Change: PR postlude generation — drop the strict-JSON requirement up front.
Effect: removes a masked retry on the publish path of every run; one less way a green run degrades at PR time.

**8. Stop ml record from mutating tracked config mid-run (graph/UX) — seed fabro-b94d**
What happened: this run's PR diff includes `.mulch/mulch.config.yaml +1` (auto-created `develop-loop` domain) plus a new expertise file — churn the reviewer had to classify before approving (reviewer journal: "Churn-only capture diff matched the changed loop files exactly").
Change: pre-create the ml domain in the toolchain image (or classify the file as expected churn in `evidence.nu`), per the seed.
Effect: seed PRs carry only seed work; smaller review adjudication surface per run.

## Smaller hygiene (same run, same evidence)

- **fabro-45bf**: stage headers lied this run — implementer prompt said "0 of 7 stages completed" with 2 done; reviewer said "2 of 7" with 5 done. Fix the progress projection to unique completed nodes. Effect: honest mid-loop progress.
- **fabro-6b58**: the in-flight check returned 15 runs / 13,519 bytes back to 2026-09-14; only this run was non-terminal. Add `created_since ≈ 48h` + explicit self-exclusion to `planner.md` step 4. Effect: ~11.5 KB less tool output per claim.
- **fabro-a701**: 6 ERROR log lines for absent optional `.codex/instructions.md` (2 per agent session init). Downgrade to info. Effect: clean warn/error view per run.
- **fabro-850f**: 4 of 7 journal lines this run are empty `data:{}` (start/tester/evidence/closeout). Skip empty writes in `stage-journal.nu`. Effect: less checkpoint/PR churn.

Not inspected: PR #169's final body content (PR state was null in the snapshot at analysis time), so I can't confirm what the retried generation produced — only that the strict-JSON failure occurred.
