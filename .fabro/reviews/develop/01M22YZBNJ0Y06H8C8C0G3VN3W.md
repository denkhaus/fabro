# Improve review — run 01M22YZBNJ0Y06H8C8C0G3VN3W

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (2.4 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-09 11:38+0000 by revisor `fabro_ask`

---

All six recommendations below are grounded in this run's actual telemetry (stage timings, token billing, tool transcripts from run events; run total: 141.6 s wall, $0.130, all stages first-pass green, seed `fabro-3c6c`, PR #88).

---

**1. Cut the planner's `sd ready` firehose — highest cost concentration in the run**
- **What happened:** The planner was the most expensive stage: $0.0687 of $0.130 total (53%), 40.4 s inference vs 2.2 s tool time. Its first tool call (`sd ready --assignee fabro --limit 200`) returned 180 seeds / 24,301 bytes (run events seq 32); the model then paged through it just to pick the top High seed. The prompt line mandating `--limit 200` lives in `.fabro/workflows/develop/prompts/planner.md` (sd command reference table).
- **Change:** Since `sd ready` is deterministic priority-ordered (fabro-5e09), make the default `--limit 15` with `--limit 200` only as fallback when nothing in the top-N is claimable. (This run is fresh evidence for already-open seed fabro-c3b4.)
- **Effect:** ~10k fewer input tokens and typically one fewer reasoning pass per planner visit — roughly 20–30% off the run's dominant cost stage, on every develop run.

**2. Reduce inter-stage checkpoint overhead — ~30% of wall time is not stage work**
- **What happened:** Active stage time sums to 95.4 s, but run wall was 141.6 s. Each of the 5 stage boundaries cost ~5 s (metadata snapshot 2.1 s + checkpoint commit + push; e.g., implementer done 11:32:44.4 → tester start 11:32:49.1), and the terminal segment (final push + PR creation) took 17 s (closeout done 11:33:23.6 → run completed 11:33:40.7). Tester/evidence/closeout are command nodes whose diffs are already captured in stage events, yet they get full snapshot+commit+push treatment.
- **Change:** In the engine/workflow checkpoint pipeline (supports open seed fabro-c2ca "Skip metadata snapshots on non-agent stages"): skip metadata snapshots on command nodes and batch pushes (push at terminal/soft-exit boundaries instead of every checkpoint).
- **Effect:** ~20–35 s (~15–25%) off every run's wall time, independent of seed size.

**3. Share the prompt-cache prefix across stages — every stage's first turn re-bills ~9k tokens**
- **What happened:** Every agent stage's first LLM turn shows `cache_read_tokens: 0` (planner seq 30, implementer seq 83; reviewer: 192) despite an identical ~8–9k token static prefix (system prompt + tools + AGENTS.md memory) — AGENTS.md (23,967 B) is loaded verbatim into all three stages. `cache_write_tokens: 0` across the whole run (run summary billing).
- **Change:** Engine-side shared cache prefix across stages of one run (this is open seed fabro-944d; this run confirms it persists on glm-5.3 with the current server 0.348).
- **Effect:** The implementer and reviewer first turns stop re-billing the static prefix at full price — approximately $0.015–0.025/run at current scale, more on multi-visit cycles.

**4. Stamp `current_seed_id` as a run label — the double-pick guard parses goal prose today**
- **What happened:** The planner's in-flight-PR guard (fabro_runs_list call, seq 41) had to extract seed ids by regex from every run's *goal text*, because all listed runs carry `labels: {}` and an identical goal string. A misparse silently disables the only double-work guard (the fabro-22e4 class the guard exists to prevent).
- **Change:** Engine: write `current_seed_id` into run labels when the planner sets it (same mechanism that already propagates the context key); optionally surface it in the `fabro_runs_list` output.
- **Effect:** The guard becomes exact-match instead of text-mining; eliminates a silent failure mode of the loop's only duplicate-work protection.

**5. Scope memory per node — the reviewer paid 42% of its input for AGENTS.md build instructions**
- **What happened:** Reviewer context breakdown (run events, stage `reviewer@1`): memory 5,810 of 13,678 input tokens — AGENTS.md cargo/just build lore, irrelevant to judging a 3-line markdown diff; it was also offered the `rust-style-guide` skill (224 tokens) for a prompt-text change. Same 23,967 B file loaded into planner (6,022 tok) and implementer (5,586 tok).
- **Change:** Add a node-level memory allowlist attribute (mirroring the existing `fs_hide`/`fabro_tools` per-node pattern in `workflow.fabro`); reviewer needs none of AGENTS.md. (Supports open seed fabro-9588.)
- **Effect:** −5–6k tokens per agent stage, shorter preambles, and fewer irrelevant-skill distractions per review.

**6. Make the implementer's `sd show` re-fetch conditional — the rule is already dead letter**
- **What happened:** `implementer.md` step 1 mandates "Re-read the seed requirements from `sd show <current_seed_id>`", but the implementer never called `sd show` (its transcript: 2 greps, 1 python3 edit, 1 verify grep) — it succeeded entirely from the planner's brief, and the reviewer approved. The mandate is ignored whenever briefs are rich, which makes the prompt contract inconsistent and leaves a rule-violation the reviewer could theoretically flag.
- **Change:** One edit in `.fabro/workflows/develop/prompts/implementer.md` (Input section): re-fetch only when the brief lacks file-level anchors or checkable acceptance bullets. (Aligns with open seeds fabro-4881/fabro-a67f; this run is clean evidence for the conditional.)
- **Effect:** Prompt matches observed correct behavior; saves one tool call + one LLM turn on brief-rich runs and removes a latent review-dispute vector.

---

**Not recommended from this run's evidence:** raising `preamble_budget_kb` (fabro-8d2c) — the 8.7 KB evidence capture reached the reviewer fully inline (no blob detour occurred); and PR skipping for journal-only diffs (fabro-9f97) — this run's diff contained real prompt changes, so PR #88 was legitimate.
