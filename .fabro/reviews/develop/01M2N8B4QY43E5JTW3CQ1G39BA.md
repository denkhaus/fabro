# Improve review — run 01M2N8B4QY43E5JTW3CQ1G39BA

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (9.4 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-16 14:16+0000 by revisor `fabro_ask`

---

Recommendations for the develop workflow, grounded in run `01M2N8B4QY43E5JTW3CQ1G39BA` (seed fabro-e6a0, 8m26s wall, $0.540 total, 0 retries, first-pass approve). Evidence sources: run events, stage journals, checkpoints, and the tracker (`.seeds/issues.jsonl`, checked for existing coverage per recommendation).

**1. Ship the nushell skill — implementer burned its biggest turn re-deriving nu 0.115 semantics. Seed: fabro-d19e (open).**
From run events: the implementer is 70% of run cost ($0.381 of $0.540, 361 s inference vs 16 s tool time), and its single largest turn (seq 94→95, ~65 s, 3,627 reasoning tokens, $0.024) was pure design deliberation over nu flag nullability, `$self` as a reserved word, and `mut x = null` typing — after which it *still* hit `nu::shell::type_mismatch` (implementer journal painpoint, recorded as mx-05fb4e). Change: add `.fabro/skills/nushell-scripts/SKILL.md` plus the one-line load rule in `.fabro/workflows/develop/prompts/implementer.md` step 2, exactly as fabro-d19e specifies. Expected effect: .nu seeds skip the probe/debug cycle (~1–2 min, $0.03–0.08 per run by the seed's own estimate) and the mut-null crash class stops recurring.

**2. Make callers actually pass `--self <run-id>` to dup-run-check.nu — the run's own deliverable is currently unusable. New seed needed.**
From run events: this run's implementer preflight invoked `nu .fabro/scripts/dup-run-check.nu fabro-e6a0` *without* `--self` (seq 81), and the run's diff touched only the script — yet the entire point of fabro-e6a0 (closed by this run after 7 documented revisor occurrences of manual trailer adjudication) is that consumers get mechanical self/foreign verdicts, which requires passing the invoking run-id. Change: update `.fabro/workflows/develop/prompts/implementer.md` step 1 and the revisor analyze Step 3.5 to call with `--self <run-id>` (the run id is already in every stage preamble header), and check in the verification battery (fabro-4cd8/45bf/a67f arms) as a fixture script so nobody improvises a fake `sd` wrapper in `/tmp` again (implementer observation 1). New-seed justification: grep of the tracker shows no open seed mentions `--self` adoption; fabro-e6a0 covered the script only and is closed. Expected effect: the revisor's per-pass manual trailer-inspection tax actually ends.

**3. Raise the reviewer's inline cap — the 16.9 KB evidence capture got blob-ref'd by a 16 KB ceiling. Seed: fabro-cf3e (open).**
From run events: `evidence@1` emitted 16,881 bytes; the reviewer node's `preamble_inline_max_kb=16` demoted it to a blob marker (visible in the reviewer@1 prompt), forcing 5 shell calls to page the blob before judging — while the reviewer's context window stood at 2.4%. Change: `.fabro/workflows/develop/workflow.fabro`, reviewer node `preamble_inline_max_kb` 16 → ≥32 (the seed's 2026-09-16 update says amend to ≥40 KB). Expected effect: captures render inline; one fewer tool round-trip per review and the unread-blob rejection class disappears. Companion backstop: fabro-7613 (open) — this reviewer performed a blob-paging workaround yet journaled `painpoints: []`, the exact silence 7613 auto-stamps engine-side.

**4. Put the implementer's check outputs in the evidence capture — the reviewer re-verified live what the implementer had already proven. Seed: fabro-d89a (open).**
From run events: the implementer's per-criterion battery (closure:self/clean, closing_evidence for fabro-45bf, never-implemented stays clean) lived only as prose PASS lines in `implementation_summary`; the reviewer then re-ran the same spot-checks itself (5 shell calls; reviewer observation: "plus live read-only spot-checks"). Change: extend `.fabro/workflows/develop/scripts/evidence.nu` to include the implementer's check transcript, per fabro-d89a (filed from the immediately preceding run on this same workflow version). Expected effect: reviewer approves from context; removes duplicated re-verification per run.

**5. Top-N the planner's `sd ready` view — 28 KB of firehose to pick line one. Seeds: fabro-66bc and fabro-c3b4 (open, same demand — implement once).**
From run events: `sd ready --assignee fabro --limit 200` returned 200 seeds / 28,098 bytes with `stdout_truncated: true` (seq 30), and the planner picked the first line of the High block (fabro-e6a0) anyway. Change: planner prompt command table in `.fabro/workflows/develop/prompts/planner.md` (PROJECT_FACTS sd table) → top-N priority-sorted invocation, full listing only as fallback. Expected effect: ~28 KB less planner context every pass — cheaper turns, no truncation noise.

**6. Bound the in-flight check and self-exclude mechanically. Seed: fabro-6b58 (open).**
From run events: `fabro_runs_list` returned 23 runs (~9 KB, all generic goals), and the planner had to reason "the only non-terminal run is this run itself" to self-exclude (planner observation 2; seq 39). Change: planner.md step 4 — pass `created_since ≈ 48h` and one explicit self-exclusion line, per fabro-6b58. Expected effect: tool output drops to ≤2 KB and the self-run confusion class (which caused a real double-implement once) disappears.

**7. Fix the pipeline-progress header — it miscounted twice in this run, post-#175. Seed: fabro-9e8b (open).**
From run events: the implementer@1 prompt read "Pipeline progress: 0 of 7 stages completed" although the planner had completed (checkpoint seq 67); the reviewer@1 prompt read "2 of 7" with start/planner/implementer/tester/evidence all done — on base 3c0a83c, which already contains the #175 dedupe fix. Change: re-verify the unique-completed-nodes computation in the engine progress projection (lib/, per closed fabro-45bf's scope) and add a regression pin. Expected effect: honest mid-run progress numbers in every stage preamble and the web UI.

**8. Structured "arms" section for multi-appendage seeds. New seed needed.**
From the planner journal: fabro-e6a0's body carried 6 dated update appendages describing 4 distinct adjudication arms, which the planner had to fold into bullets by hand. Change: seed-authoring convention (revise intake + `docs/agents/issue-tracker.md`) requiring a structured arms list once a seed accumulates a second dated occurrence-appendage. New-seed justification: existing authoring seeds cover body-presence (fabro-7f27), contradiction lint (fabro-7773), and basis-ref lint (fabro-3839) — none covers a structured arms schema. Expected effect: brieves stop re-deriving arm structure from prose; lower misparse risk on the highest-priority seeds.

Not recommended despite checking: gate changes (tester ran 3.6 s, "no crates touched", zero duplication with the implementer's `just verify implementer` — the existing gate-ban design worked), and reason-by-1 reasoning-effort tuning (already `low` on all three agent nodes; the cost is in tool-semantics knowledge gaps, i.e. rec 1, not effort).
