# Improve review — run 01M2VDR9YH2T6PFCEWJ7E3B0KS

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (5.3 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-18 23:41+0000 by revisor `fabro_ask`

---

All evidence is gathered (run events, stage journals, worker logs, and the seed tracker). Run recap for grounding: seed **fabro-c77a** claimed → implemented (+33/−2 in one file, `.fabro/workflows/revisor/prompts/file.md`) → gate green in 6.2s → approved → closed; PR #248. Total 5m16s, $0.362. Stage split (from run events): planner $0.075/36.8s, **implementer $0.240/190.9s (66% of cost, tool time only 2.1s)**, reviewer $0.047/23.4s. Everything below is from this run's events/journals/logs unless noted.

## Recommendations (by expected impact)

**1. Make the evidence capture survive the stage renderer — the reviewer's "56 lines omitted" fallback. → seed fabro-meta-c9f2 (open, P1)**
- What happened: the evidence output was only 8,577 B, yet the reviewer's preamble rendered its stage section truncated ("56 lines omitted"), hiding the head of the only changed file's diff. The reviewer fell back to 2 shell calls (read full `file.md`, re-derive diff scope vs claim base `d163f390`) — its journal explicitly asks for a budget raise "for single-file prompt-asset seeds (run 01M2VDR9YH2T6PFCEWJ7E3B0KS)". This is occurrence 5 of the class the seed already documents (occurrences 1–4 listed in its body).
- Change: implement the seed — rearchitect `.fabro/workflows/develop/scripts/evidence.nu` to emit verdict-critical content diff-first within the renderer's tail window, and/or cap the engine-side per-stage render for the evidence section only.
- Expected effect: reviewer approves from context in one pass on every seed; removes the per-review shell detour and the verification-blocked-cycle risk that previously burned whole runs.

**2. Bound the planner's in-flight check and stop feeding it null PR states. → seed fabro-6b58 (open)**
- What happened: the planner's `fabro_runs_list` call was unbounded — 66 runs returned — and PRs #243/#244/#246 showed `pull_request.state: null`, forcing the model to reason it out (its reasoning trace: "PR state null doesn't prove open… Proceed"). That's judgment exactly where a deterministic guard should decide; a wrong call here costs a duplicate run (~$0.36/5min today, historically ~15min/$0.73 per fabro-6b58's evidence).
- Change: implement fabro-6b58 in `.fabro/workflows/develop/prompts/planner.md` step 4 — `created_since ≈ 48h` plus the self-exclusion line — and add this run's facet to the seed: the projection's PR-state enrichment must either resolve or omit, never emit `null` for live PRs (planner spent its reasoning round dismissing them).
- Expected effect: tool output drops from ~66 runs to a handful, one less reasoning round per planner pass (~8s tool time here), and the double-pick guard stops depending on model tolerance of ambiguous data.

**3. Kill the implementer's redundant `sd show` re-fetch mechanically, not with prose. → seed fabro-4881 (open; fresh evidence)**
- What happened: despite a complete brief (the planner's journal even said "treat the body's decision block as the spec"), the implementer's second tool call was `sd show fabro-c77a` — the exact dead-letter recurrence fabro-a67f documented before closing. The implementer was 66% of run cost ($0.24, 188s inference, 15 messages) for a one-file markdown edit.
- Change: implement fabro-4881 structurally — have the planner stage stamp the full seed body into a context key rendered via the implementer's `preamble_allow_keys`, and delete the `sd show` sentence from step 1 of `.fabro/workflows/develop/prompts/implementer.md` entirely (prose conditionals have now failed twice).
- Expected effect: −1 tool call and −1 LLM round per implementer pass on every seed; on this run's shape ~10–15s and a few cents, more on Rust seeds where re-imported ambiguity historically caused debug cycles.

**4. Make `rg -rn` mechanically impossible, not prompt-forbidden. → new seed needed.**
*Justification: the prompt-line fix (closed fabro-2eb6) is landed in implementer.md, yet the trap recurred in this run — no existing seed proposes a toolchain-level guard.*
- What happened: implementer journal records an exploratory `rg -rn "arms"` whose display garbled to literal 'n' (a near-miss; the prior occurrence rewrote a function signature). The worker log adds a compounding error-handling gap: 8× `unsupported_control` warnings — the zai protocol drops the tool-result error flag — so the implementer's one errored shell call reached the model without an error status.
- Change: a tiny `rg` wrapper in the toolchain image (`.fabro/Dockerfile.toolchain` — already slated for one rebuild by fabro-c643, so ride that rebuild) that hard-errors on combined `-r`+`-n` with a one-line explanation.
- Expected effect: eliminates the misdiagnosis class at the source and guarantees a loud error even where the provider swallows error flags.

**5. Own the ADR-0022 post-landing checkpoint — the filing balance can starve the backlog silently. → new seed needed.**
*Justification: the ≥1-week re-measurement was parked inside fabro-c77a's body, which this run closed; no open seed owns the watch (tracker grep for balance/ADR-0022/drain finds nothing open).*
- What happened: the implementer's own journal predicts the shipped behavior: "a zero-credit revisor pass files zero non-exempt seeds by design… expect `balance: 0 / 0` with overflow observations." Nothing mechanical re-checks whether findings start dying in overflow permanently.
- Change: file a line-watch seed: re-measure creates:closes ~1 week post-landing (≈2026-09-25); escalate if N consecutive revisor passes file zero non-exempt seeds while overflow observations accumulate.
- Expected effect: the loop notices starvation of intake instead of discovering it weeks later as an empty tracker — the direct UX guard for the change this run shipped.

**6. Fix the one-character anchor-path truncation in the preflight verdict table. → seed fabro-e8ae (open; second occurrence)**
- What happened: this run's preflight again flagged fabro-c643 with path `.fabro/Dockerfile.toolchai` — one character short of the seed body's `.fabro/Dockerfile.toolchain`, the identical truncation fabro-e8ae filed from the previous run. Any planner adjudicating that row gets corrupted data.
- Change: fix the anchor-path extraction in `.fabro/workflows/develop/scripts/planner-preflight.nu` per the seed.
- Expected effect: verdict-table paths match seed bodies; flagged candidates get adjudicated on real data.

**Minor observation (no rec without more evidence):** the PR title/description generation (`zai:glm-4.7`, strict JSON) failed once and retried loose — it recovered, so PR #248 is fine; and 6 log ERRORs from a missing `/workspace/fabro/.codex/instructions.md` at every session init are pure noise. Neither affected the outcome; I did not inspect PR #248's remote state (projection shows `state: null`).
