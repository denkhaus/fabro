# Improve review — run 01M2YC82ECXM5KY8EC1TPNPRDK

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (5.3 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-20 03:14+0000 by revisor `fabro_ask`

---

All evidence gathered (run events seq 1–203, checkpoints, gate output, journal records, tracker sweep). Context for the recommendations: this run was a clean first-pass cycle — 298.7 s wall, $0.273 total, zero retries, reviewer approved with 0 tool calls off a 5.9 KB inline evidence capture. The costs that remain are concentrated in the **planner** ($0.197 = 72% of run cost, 139.3 s inference, 9 tool calls) and in **gate-warning noise**. Recommendations, ordered by expected impact:

---

**1. Stop re-adjudicating the externally-blocked top seed every lap (fabro-af22)**
- **What happened (run events seq 50–88):** the planner burned 6 LLM rounds + 4 shell probes (~98 s wall, ~$0.12 of its $0.197 — roughly 40% of total run cost) re-deriving that priority-1 seed fabro-af22's fix lives in the rev-pinned `pebble-coding-agent` dep (`Cargo.toml` line 136) behind open upstream PR #784, before skipping to fabro-7028. Its own journal says it "will keep surfacing as top candidate every lap" — this is a per-lap tax, and the 4.5 s preflight table (`output.preflight`) marks af22 `clean` each time without carrying the blocker.
- **Change:** (a) now, zero-code: unassign or block seed **fabro-af22** until #784 merges upstream (the journal of this run asks for exactly this); (b) durable: add an arm to `.fabro/workflows/develop/scripts/planner-preflight.nu` (beside arms 1–3 in its STANDING POLICY header) flagging candidates whose body carries an upstream/close-external marker (`UPSTREAM PR OPEN`) as `external_block: true`, so the planner skips on the table instead of re-probing.
- **Expected effect:** −6 LLM rounds / ~$0.10–0.12 per lap while af22 stays top (~40% of this run's cost), and every future externally-parked seed skips for free.
- **Seed:** names **fabro-af22** for action (a); for (b) — *new-seed justification: no open seed covers mechanical external-blocker flagging — fabro-d9f9 (open) only adds a "don't probe cargo checkouts" prompt bullet, and fabro-32db (closed) was the in_flight misclassification, a different arm.*

**2. Authoring lint: cross-check `output.*` allow-keys against command-node ids (generalize the fabro-7028 class)**
- **What happened:** this run *fixed* fabro-7028 — a declared key (`output.gate_known_bug_hits`) that the engine never produces because it derives `output.<node-id>` — a bug that had fired a live engine warn for days before anyone connected it. The sibling symptom fired again *in this same run*: engine warn at seq 99, `context_allow_keys dropped: output.planner` (the planner's own response-dedup key isn't declared in its allow list either). The implementer's journal this run proposes precisely this lint.
- **Change:** add a check to `scripts/validate-workflows.nu` (already runs the baked-in `fabro-validate`, wired into `just qualitygate`): every `output.<x>` in any node's `context_allow_keys`/`preamble_allow_keys` must resolve to a declared command-node id (or an agent node's own dedup key), else gate-red.
- **Expected effect:** the inert-enrichment / blind-stage class (fabro-7028; the fabro-0a4c blind-implementer incident) is caught at authoring time, not after days of live warns.
- **Seed:** *new-seed justification: closed fabro-7028 fixed the single instance and closed fabro-a211 documented routing activation — no open seed covers the graph-level allow-key↔node-id cross-check; it currently lives only in this run's implementer journal, and journal-only requirements don't exist.*

**3. Teach prompt-lint that the develop/conductor schemas are *intentionally* routing-active**
- **What happened (tester output, this run):** `prompt-lint: ok — 43 files, 11 warnings` — 10 of the 11 are `top-level property 'preferred_next_label'/'outcome'/… is routing-named` against `.fabro/workflows/develop/schemas/planner-output.schema.json` and the conductor schema, both deliberately routing-shaped (fabro-9ec3 arm 3 keeps the routing contract by design; the schema description says so), plus 1 date-pin nag. Every green gate run pays this noise, which trains readers to ignore the channel the lint exists for.
- **Change:** in `.fabro/scripts/prompt-lint.nu`, suppress the routing-named warning for schema files that declare their routing intent (match the existing description text, or add an explicit `"x-routing-intent"` marker both schemas set); keep it firing for accidental opt-ins.
- **Expected effect:** gate output drops 11 → ≤1 warning; genuine routing-activation surprises (closed fabro-a211's purpose) become visible again.
- **Seed:** *new-seed justification: closed fabro-a211 ADDED this warning; no open seed covers the intentional-routing allowlist — open fabro-8275 is the engine-side fidelity warn level, a different emitter.*

**4. Bound the planner's in-flight and candidate inputs — fabro-6b58 + fabro-c3b4**
- **What happened (seq 45, 51–58):** `fabro_runs_list` returned **105 runs** with full goals and mostly-null PR states — the planner's reasoning visibly fumbled it ("306 null — merged presumably", "no non-terminal runs besides current") inside a $0.055 round; `sd ready` poured ~9.5 KB of full seed bodies just to learn the queue order. This run got lucky; the guesswork over null states is exactly where a mis-skip or double-pick would start.
- **Change:** implement open seed **fabro-6b58** (call `fabro_runs_list` with `created_since ≈ 48h` + one self-exclusion line in `planner.md` step 4) and open seed **fabro-c3b4** (top-N `sd ready` view, full listing only when top candidates are unclaimable).
- **Expected effect:** ~15–25 KB less planner context per pass, faster first output, and the null-PR-state deduction this run performed by hand becomes unnecessary.
- **Seed:** **fabro-6b58** and **fabro-c3b4** (both open; this run is fresh evidence for both).

**5. Ship the planner footgun one-liners — fabro-6997**
- **What happened (seq 59–71):** the planner ran `rg -rn "find_skill_references" lib/` — the exact documented anti-pattern (`-r n` = replace-with-literal-n, from closed fabro-2eb6) — got an empty result, then reasoned "rg output maybe empty or hidden?" and spent two follow-up greps recovering. The rule lives only in `implementer.md`; the planner prompt never got it.
- **Change:** implement open seed **fabro-6997**: two one-liners in `.fabro/workflows/develop/prompts/planner.md` — never `rg -rn` (`-n` alone is line numbers); glob on fs_hide paths returns empty, go straight to shell.
- **Expected effect:** −2 tool calls and −1–2 LLM rounds (~$0.02–0.05) per planner pass touching platform paths; this run demonstrably paid it.
- **Seed:** **fabro-6997** (open since 2026-09-09; this run is a fresh recurrence).

---

**Not recommended for change (verified good this run):** the evidence pipe (5.9 KB capture rendered inline under the 16 KB reviewer budget — reviewer needed 0 tool calls), the deterministic spine (preflight 4.5 s, claim_check 82 ms, evidence 305 ms, closeout 386 ms), and the gate itself (24.3 s, "no crates touched" path). 

**Evidence basis:** run events seq 1–203 and checkpoints (timings, per-stage cost, planner tool transcript), tester gate output (prompt-lint warnings), stage journals from `.fabro/journal/01M2YC82ECXM5KY8EC1TPNPRDK.jsonl`, and the tracker sweep of `.seeds/issues.jsonl` plus `.fabro/workflows/develop/scripts/planner-preflight.nu` for seed-coverage checks.
