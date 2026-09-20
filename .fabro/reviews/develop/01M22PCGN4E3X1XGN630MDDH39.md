# Improve review — run 01M22PCGN4E3X1XGN630MDDH39

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (2.9 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-09 09:08+0000 by revisor `fabro_ask`

---

Grounded in run 01M22PCGN4E3X1XGN630MDDH39 (seed `fabro-fc1b`, a one-line edit to the implementer prompt; succeeded in 2m53s wall, $0.145 LLM cost, 1 visit per node, PR #78). All facts below are from run events/checkpoints and the stage prompts in the graph spec.

## Recommendations, ordered by expected impact

**1. Kill the `sd ready` firehose the planner pays on every fresh pass**
- Evidence: planner's first tool call (`sd ready --assignee fabro --limit 200`, event seq 31��33) returned **176 issues / 23.7 KB, flagged `stdout_truncated: true`**; only the top High candidate was ever used. That blob drove the planner's first LLM round to 12,240 input tokens. Planner stage: 34.5s / $0.053 — 37% of run cost for a trivial claim.
- Change: `.fabro/workflows/develop/prompts/planner.md` (sd table, step 1) — render a top-N view by priority (e.g. `sd ready ... --format json` piped to a bounded renderer, or `| head -30` guidance) instead of the full listing. (Open seed fabro-c3b4 proposes this; this run is fresh evidence.)
- Effect: ~5–6k tokens and several seconds off every fresh planner pass; no more truncated tool output as planning input.

**2. Make the implementer's `sd show` re-fetch conditional on brief quality**
- Evidence: the planner's brief already contained all five acceptance criteria nearly verbatim, yet implementer call #1 (seq 78) re-ran `sd show fabro-fc1b` (~1.7 KB re-delivered) plus a 60-line head of the very prompt file — one extra tool round and LLM round. The implementer was the **largest stage: 66.6s / $0.061 (42% of cost) for a 1-line diff**, 62.9s of it inference.
- Change: `.fabro/workflows/develop/prompts/implementer.md` step 1 — "If the brief carries bulleted acceptance criteria covering the seed body, skip the `sd show` re-fetch; fetch only when the brief is thin, ambiguous, or a re-plan." (Open seeds fabro-4881/fabro-a67f.)
- Effect: one tool call + one model round (~6–10s, ~$0.01) removed per implementer pass.

**3. Planner observations naming consistency work must become brief bullets**
- Evidence: the planner journaled *"the same rule is referenced again at line 78 … which the implementer should keep consistent"* — but put no such bullet in the brief. The implementer (correctly) kept the mechanical-rewrite paragraph verbatim, which still reads *"verify with ONE focused check (compile check or ONE focused test, as above)"* — now **contradictory** with the new tests-touched rule this very seed introduced. A latent inconsistency shipped through an approving review because the reviewer checks bullets, not journals.
- Change: `.fabro/workflows/develop/prompts/planner.md` step 6/7 — any observation that names required consistency/scope work must be folded into `current_seed_brief` as an explicit bullet (or explicitly waived in the brief).
- Effect: prevents self-contradictory prompt text surviving the loop; zero-cost correctness win.

**4. Document the `ml record` flag contract per record type**
- Evidence: implementer's first lesson-capture call **failed**: `Error: convention records are missing required flag(s): --content` (seq 96–97), costing a wasted call, an LLM retry round, and — notably — `ml record` **auto-created the `workflow` domain, mutating `.mulch/mulch.config.yaml`** (`+ workflow: {}`) as surprise churn the reviewer then had to classify. The implementer journaled the flag surprise.
- Change: `.fabro/workflows/develop/prompts/implementer.md` step 6 / Lesson-capture section — one line per record type listing required flags (convention needs `--content`). (Open seed fabro-96bd; this run is the proof.)
- Effect: eliminates a deterministic failed call + retry per lesson capture; ideally suppress domain auto-creation writing to tracked config mid-run.

**5. Deliver the evidence capture whole — the reviewer's stage view was cut**
- Evidence: in reviewer@1's prompt, the evidence stage Output begins with **"(14 lines omitted)"** — the integrity header and seed-work file list were elided by the stage renderer; the `command.output` context key never rendered in the `## Current context` table. The reviewer worked around it by re-reading `implementer.md:25` at HEAD via shell (its journal admits this). The workaround is cheap when it works; the failure mode (`Verification blocked`) is not.
- Change: `.fabro/workflows/develop/workflow.fabro` evidence node — add `output_schema` + `context_allow_keys` and swap the reviewer's `preamble_allow_keys` from `command.output` to `output.evidence` (open seed fabro-e4c4), or lift the summary:high tail-lines cap (fabro-meta-c9f2).
- Effect: capture arrives inline and complete; one shell detour removed per review; the truncated-capture rejection class disappears.

**6. Stop the monotonic war-story growth of implementer.md step 4**
- Evidence: step 4 is a **single ~2.3k-char line** carrying four historical run citations (fabro-0d56, 01M1YJ8R…, 01M11P68SHF, and now 01M20T9S8J added by this run). Every implementer pass re-reads it; this run's implementer even spent a reasoning round on meta-confusion (*"Wait — I AM the implementer running this prompt"*, seq 83) because the seed edits the file the agent is executing.
- Change: `.fabro/workflows/develop/prompts/implementer.md` — keep the operative rule to 3–4 sentences and move run-citation justifications to a footnotes section or the `ml` expertise store (mx-e26272 already captures this run's rationale durably). Add one line for platform-targeting seeds: "you edit the source file; your own instructions this pass are already rendered — no recursion."
- Effect: smaller, faster implementer prompts every run; removes the self-reference confusion round.

**7. Fix two deterministic false signals in stage headers**
- Evidence: (a) implementer's prompt said **"Pipeline progress: 0 of 7 stages completed"** when start+planner were done; reviewer's said "2 of 7" when five nodes were complete (open seeds fabro-45bf/fabro-a0e3). (b) The only WARN in the worker log: `preamble_allow_keys entry absent from context node=implementer key=output.gate_known_bug_hits` — fires on **every green first visit** because that key only exists on gate-red bounces (open seed fabro-8275).
- Change: engine preamble renderer (progress denominator = unique completed nodes) and downgrade the by-design allow-key absence to info.
- Effect: honest progress in every stage prompt; warn channel reserved for real problems.

**8. Path-scope the gate for prompt-only seeds**
- Evidence: tester ran `just qualitygate` → 5.4s, 87 bytes: *"no crates touched / format clean"* — near-zero assurance for a markdown-only diff, and on a cold cache the same recipe costs minutes (open seed fabro-01b9).
- Change: `justfile` `qualitygate` recipe or the tester node script — when the run diff touches no crate sources, skip compile/clippy/nextest and run format + config checks only.
- Effect: cheaper, faster, honest gating for the loop's frequent prompt-revision seeds.

**Not worth changing based on this run:** the cycle guards, gatebounce, closeout (0.3s, deterministic, closed exactly `fabro-fc1b` via stdin), and the in-flight PR guard (`fabro_runs_list` worked, excluded nothing correctly) all behaved exactly as designed. Cache reads were healthy (191k cache_read tokens); only write-side sharing is absent (fabro-944d), which matters more on multi-cycle runs than on this one-pass one.
