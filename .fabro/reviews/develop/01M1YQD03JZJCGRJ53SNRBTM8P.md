# Improve review — run 01M1YQD03JZJCGRJ53SNRBTM8P

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (2.8 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-07 20:09+0000 by revisor `fabro_ask`

---

All evidence below is from this run's events (stage timings, tool transcripts, billing), the worker log (2 warnings), and the stage journal. Run shape: 7 stages, 1 visit each, 0 retries; wall 137 s; $0.1368 total; planner $0.0579 (42%), implementer $0.0481, reviewer $0.0307.

## Recommendations, by expected impact

**1. Share the prompt cache across stage sessions — cut ~30% of run cost.**
What happened: every agent stage opens a fresh session and its first LLM turn is fully uncached — planner turn 1: 11,204 input tokens, `cache_read_tokens: 0`, $0.016; implementer turn 1: 11,418 uncached, $0.0166; reviewer turn 1: 13,689 uncached, $0.0197 (64% of that stage's bill). That's ~36k uncached tokens ≈ $0.04–0.05 of the $0.137 total, mostly re-reading identical AGENTS.md memory (5.5–5.8k tokens each), tool schemas (~1.6k), and system prompt. Billing shows `cache_write_tokens: 0` for the entire run.
Change: engine prompt-cache layer (already filed as seed fabro-944d) — write a shared cache prefix for the memory/tools/system blocks so stage 2 and 3 hit cache on turn 1.
Effect: ~25–30% cost reduction per run with zero behavior change.

**2. Cap the planner's `sd ready` firehose.**
What happened: planner's first tool call returned **21,093 bytes / 158 ready issues** (`stdout_truncated: true`, event seq 33). The planner needed the top High-priority candidate; it carried ~5.5k tokens of Medium/Backlog listing through all 7 remaining turns and had to blindly trust tool ordering to pick.
Change: `.fabro/workflows/develop/prompts/planner.md`, sd command table row 1 — replace the full listing with a top-N view (`sd ready --assignee fabro --limit 200 | head -40`, keeping the anti-truncation rationale of fabro-c16d), fall through to the full list only when the top-N is exhausted or all skipped.
Effect: shorter planner context on every turn, fewer planner turns, less mispick risk; planner is the costliest stage ($0.058, 42%).

**3. Stop snapshotting the meta branch after command stages — ~11% wall time.**
What happened: 8 metadata snapshots at 1.6–2.1 s each (init 2,087 ms; after planner 1,789; implementer 1,995; tester 1,745; evidence 1,751; reviewer 1,776; closeout 2,029; finalize 1,622) ≈ **14.8 s of the 137 s wall**, in-line between every stage — including after `tester`, `evidence`, `closeout`, which produce no agent-session state worth snapshotting synchronously.
Change: engine checkpoint path (filed as fabro-c2ca / fabro-cf03) — make snapshots async, or snapshot only after agent stages plus finalize.
Effect: ~5–8 s faster runs; no information loss for command stages.

**4. Harden PR-postlude generation — it failed and retried on this run's terminal path.**
What happened: worker log, 20:04:50: `PR content structured generation failed; retrying once without strict JSON output ... trailing comma at line 4 column 3`. The glm-4.7 title/body call emitted fenced JSON with a trailing comma; the strict parse died; a fallback retry saved PR #49 (created 20:04:56, ~14 s after exit).
Change: engine `pipeline::pull_request` (matches open seed fabro-6a5a) — strip code fences/trailing commas before parsing, or request plain-text title line + body instead of strict JSON.
Effect: deterministic PR creation; removes a model-output-dependent failure point from the publish path of every run.

**5. Teach `evidence.nu` to classify the engine's own journal file.**
What happened: reviewer journal observation — its `git diff` showed `.fabro/journal/<run>.jsonl +3` beyond the evidence churn list, because the stage-journal hook writes after the capture; the reviewer had to spend verification budget rationalizing an unexplained diff line (exactly the artifact it's trained to distrust) instead of judging the seed.
Change: `.fabro/workflows/develop/scripts/evidence.nu` — add `.fabro/journal/**` (and `.fabro/blobs/**`) to the known-transient/churn section with an explicit "(engine journal, expected)" label.
Effect: every review sees a reconciled churn list; eliminates a recurring false-deviation risk.

**6. Gate the implementer's mandatory `sd show` re-fetch on brief quality.**
What happened: implementer step 1 says "Re-read the seed requirements from `sd show`" — it re-fetched fabro-d0c7 (1.4 KB output, one extra LLM round) even though `current_seed_brief` already contained all 7 acceptance criteria verbatim, target file included. Open seed fabro-4881 names exactly this.
Change: `.fabro/workflows/develop/prompts/implementer.md` step 1 — "Run `sd show <id>` ONLY if the brief is thin (missing criteria, ambiguous, or marked verification-only)."
Effect: one fewer tool call + LLM round (~10–15 s, ~$0.017 of the implementer's cold first turn) per pass.

**7. Make `glob` under fs_hide fail loudly, and hoist the fs_hide rule in the planner prompt.**
What happened: planner wasted a full turn discovering the boundary: `grep` on `.fabro` correctly errored ("hidden from this stage by fs_hide"), but `glob` for `.fabro/workflows/**/*planner*` returned a **silent empty result** (`is_error: false, output: ""`) — indistinguishable from "no match". One LLM round + 2 tool calls burned despite the warning already buried mid-step-3 prose.
Change: engine agent tool layer — make hidden-path `glob` return the same fs_hide notice grep does; in `planner.md`, move the "fs_hide binds file tools; use shell for `.fabro/**`" rule from inside step 3 to the tool-notes area near the top.
Effect: eliminates one wasted inference round on every platform-path seed (the most common seed class in this tracker).

**8. Two one-line hygiene fixes surfaced by this run's log and diff.**
(a) Worker log 20:02:57: `WARN ... preamble_allow_keys entry absent ... node=implementer key=output.gate_known_bug_hits` — this fires on every first-visit implementer because the key only exists after a gate-red bounce (by design; seed fabro-8275). Demote to info in `fabro_workflow::lifecycle::fidelity` when the producing node (`gatebounce`) wasn't visited — WARN noise trains operators to ignore real warnings.
(b) This run's diff incidentally fixed the planner.md duplicate-"4." numbering — but open seed fabro-890d ("Renumber the duplicated step 4") is still open in the tracker; nothing marks sibling seeds superseded when a merged change satisfies them. Add a post-merge sweep (improve workflow or a closeout.nu extension) that greps the merged diff against open seeds targeting the same files. Effect: tracker stays truthful; the next planner doesn't claim done work — which is precisely the double-pick class this run's seed was filed against.
