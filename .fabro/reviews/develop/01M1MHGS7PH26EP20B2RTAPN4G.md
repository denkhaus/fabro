# Improve review — run 01M1MHGS7PH26EP20B2RTAPN4G

- workflow: develop
- branch integrated: denkhaus-lab
- status: succeeded (completed), 10.3 min, $0.678809
- generated: 2026-09-03 23:22+0200 by `fabro ask` with `scripts/prompts/improve.md`

---

# Improve review — run 01M1MHGS7PH26EP20B2RTAPN4G

**What happened (from run events):** 12m44s wall, $0.679, two seeds closed (fabro-93f2 `-step`, then fabro-87cf `-last` via a second cycle), both first-pass approvals, exit "Tracker empty". One failure: tester@1 went red on gofmt. Cost is concentrated in the implementer: $0.556 of $0.679 (82%), ~400s of 547s active. Recommendations ordered by expected impact.

---

### 1. Give the implementer a non-gating formatter pre-flight
- **Evidence:** tester@1 failed solely on `gofmt check: unformatted files: fib_test.go` (8 lines of hand-aligned struct whitespace, from run events and worker log, 21:11:27). The fix cost a full extra cycle: implementer@2 (53s, $0.033), a second gate run, an extra commit. implementer@2's journal painpoint asked for exactly this fix.
- **Change:** `.fabro/workflows/develop/prompts/implementer.md`, "## Your job this pass" step 4 currently says "nothing that formats, lints, or runs the full suite". Carve out one exception: *"Do run the project's formatter in check mode (e.g. `gofmt -l .`) and fix what it lists before finishing — that is a pre-flight, not the gate."*
- **Expected effect:** eliminates the one avoidable failed-cycle class observed in this run (~1 min + $0.03 + a commit per occurrence); the gate red on a 10-minute run was 100% whitespace.

### 2. Close the expertise loop — recall, not just record
- **Evidence:** The lesson contract works on the write side (mx-7eaa5a, mx-e40184, mx-e96d5a recorded; `.mulch/expertise/gofib.jsonl` grows in every checkpoint diff). But no stage ever ran `ml recall`/`ml prime`: planner@1's reasoning trace considered it ("as instructed in AGENTS.md") and then skipped it, and every `agent.memory.loaded` event shows only `AGENTS.md`. Meanwhile implementer@3 (-last, 181s/$0.288) re-derived the exact pattern mx-7eaa5a had recorded one seed earlier, and mx-e40184 ("gofmt -w after hand-editing struct literals") was recorded only *after* the failure it describes.
- **Change:** implementer.md step 1 — replace "Re-read the seed requirements from `sd show`" with "Run `ml recall` for the project domain first; reuse recorded patterns (name the mx-ids you applied). Fetch the full seed only when the brief is thin or flags ambiguity." This also fixes the existing contradiction between "## Input" ("the brief is authoritative") and step 1 (re-read the seed anyway — both implementer visits ran a redundant `sd show`).
- **Expected effect:** later seeds start from recorded patterns instead of rediscovering them; in this run -last could have been cheaper than -step instead of costing more, and the gofmt lesson would exist *before* the gate, not after.

### 3. Handle the stale goal line in multi-seed runs
- **Evidence:** reviewer@2 journal painpoint: the goal text named fabro-93f2 (`-step`) while the in-progress seed, spec, evidence, and closeout were all fabro-87cf (`-last`); it "had to reconcile this manually and trust the capture's spec". `graph.goal` is a static context value, so every stage after the first closeout sees a wrong-seed goal.
- **Change:** the real fix is engine-side (refresh the goal per cycle or stamp the current seed id into the goal header — already journal-filed). Graph-side mitigation now: one line in `prompts/planner.md`, `prompts/implementer.md`, and `prompts/reviewer.md`: *"The goal names this effort's first seed; after any closeout, `current_seed_id` and the capture's in-progress spec are authoritative."*
- **Expected effect:** removes per-reviewer manual reconciliation and the risk of a wrong-spec verdict in runs with 3+ seeds, where the goal text gets progressively more misleading.

### 4. Fix PR body generation (it silently fell back this run)
- **Evidence:** worker log 21:19:43: `pull_request: PR content generation failed; using deterministic fallback title and skeleton body model=glm-4.7 error=LLM generation failed: No object generated: Failed to parse response as JSON`. PR #12 therefore shipped a skeleton body.
- **Change:** run settings `pull_request.model` is `zai:glm-4.7`; point it at `zai:glm-5.3` (which parsed routing JSON correctly in all 8 agent completions this run), or relax the JSON requirement for PR prose.
- **Expected effect:** real PR title/description instead of a skeleton; config-only change.

### 5. Deduplicate repeated command-stage sections in preambles
- **Evidence:** reviewer@2's prompt (from run events) contains the identical green-gate output **twice** and the evidence preview **twice**; planner@2's preamble repeats the tester section the same way. The graph comment in `workflow.fabro` already flags this: "Residual duplication (per-visit stage sections) is engine-side dedup work".
- **Change:** prioritize that engine seed — collapse identical command-node outputs across visits (keep latest, or latest-per-outcome).
- **Expected effect:** preambles stop growing linearly with seeds per run; harmless at 2 seeds/24KB, but it is pure waste re-read by every downstream stage.

---

**What worked — keep as-is (from run events/journals):** per-seed evidence scoping (fabro-7e44) is verified in production — capture@2 was 18.3KB containing only `-last` hunks, and reviewer@2 verified end-to-end from the blob ref; reviewer at `reasoning_effort=low` costs $0.027–0.031/visit with both reviews first-pass approvals; the deterministic closeout + cycle guards routed cleanly through two full loops with zero model-compliance dependence.
