# Improve review — run 01M2VX6PVR87WS6XJQNY6YMMFE

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (6.3 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-19 04:14+0000 by revisor `fabro_ask`

---

# Run 01M2VX6PVR87WS6XJQNY6YMMFE — improvement recommendations

**What happened (from run events/checkpoints, journal, worker logs):** clean single-pass cycle on docs seed `fabro-a211` — preflight 4.2s → planner 103s/$0.220 → implementer 212s/$0.217 (2 shell errors, lesson `mx-dd7b40`) → gate green 6.0s → evidence 0.3s → reviewer 19s/$0.039 (Approved) → closeout 0.4s. Total 6m07s, $0.475, PR #260. Zero retries, zero bounces; planner + implementer = 92% of cost. The machinery worked — the recommendations below target residual friction, ordered by expected impact.

---

**1. Resolve ambiguous in-flight marks instead of silently skipping the top candidate**
- **Change:** one sentence in `.fabro/workflows/develop/prompts/planner.md`, step 4 (IN-FLIGHT EXCLUSION): an `in_flight` mark whose runs-list PR state is null/unknown must be resolved with ONE `web_fetch` of the PR page (the exact affordance this run's planner already used for upstream fabro-sh#784) before skipping — merged/closed PR + terminal run ⇒ candidate is claimable; journal the resolved state.
- **Grounding:** the planner skipped `fabro-c643` — the FIRST-listed, P1 candidate (cargo-chef build-cache, its own seed body claims "4–7 min off every develop run") — journaling verbatim: *"in-flight run 01M2TWKB0M27E37BXX6ZDP0X8E (PR #243 state unresolved in runs list)"* (planner journal record, run events). The run then took a P2 seed. If #243 is merged, this P1 is being withheld every single run on ambiguous data. Nano-fix while touching it: the preflight's `anchor_flags` shows the seed body cites a rotted path `.fabro/Dockerfile.toolchai` (real file: `.fabro/Dockerfile.toolchain`) — record the correction via `sd update --description` pre-claim.
- **Expected effect:** recovers the highest-value claimable candidate; removes a recurring per-run opportunity cost.
- **Seed:** new-seed justification — no existing seed covers null-PR-state adjudication (fabro-9372 is engine-side marking, fabro-06e0/91ff built the exclusion, fabro-22e4 was the original duplicate-claim; none covers the ambiguous-state arm).

**2. Move externally-gated skip adjudication from the planner LLM into the preflight script**
- **Change:** extend the open seed **fabro-ead4** (already extending the same verdict table in `.fabro/workflows/develop/scripts/planner-preflight.nu`) with an `externally_gated` arm: `http get` the upstream PR state (public repo, unauthenticated) + `git merge-base --is-ancestor` check, emitted per candidate.
- **Grounding:** the planner re-derived `fabro-af22`'s park from scratch this run — upstream PR state via `web_fetch` plus ancestry shell checks — journaling: *"fabro-af22 (High bug) skipped: externally gated — upstream fabro-sh/fabro#784 still Open and the local merge commit fb9b39a0 … is NOT an ancestor"*. That is deterministic, mechanically-checkable fact (fabro-9ec3's own standing policy says such rules belong in the preflight script, never in prose), and it will be re-derived **every run** while af22 (P1) sits open. Planner was the joint-costliest stage: 103s, $0.220, 45.5k conversation tokens, 12 tool calls.
- **Expected effect:** cuts planner tool calls/tokens/latency every run; removes a network-dependent `web_fetch` from the claim critical path.
- **Seed:** fabro-ead4 (open, same file/mechanism — add the arm there rather than a new seed).

**3. The reviewer's evidence head was STILL truncated after the fabro-meta-c9f2 fix**
- **Change:** raise `preamble_output_max_lines` 200→400 on the reviewer node in `.fabro/workflows/develop/workflow.fabro` (and/or bound loop-churn diff rendering in `.fabro/workflows/develop/scripts/evidence.nu` — churn diffs are secondary material).
- **Grounding:** the reviewer's preamble rendered the evidence section starting `"(59 lines omitted)"` — the integrity header, seed-work file list, and the head of the churn diffs were invisible (from the reviewer stage prompt, run events). This is the 5th occurrence of the class that fabro-meta-c9f2 tracked (occurrences 1–4 caused wrongful rejections), and the **first post-fix**: c9f2 closed 2026-09-19 00:54 with the 200-line raise; this run's mixed capture (~259 lines: 2 docs files + 4 loop-churn files) exceeded it. The reviewer coped via one shell `rg` fallback — exactly the detour the fix was meant to eliminate.
- **Expected effect:** reviewer sees the full capture inline; zero shell fallbacks; eliminates the wrong-rejection risk class for larger mixed captures.
- **Seed:** new-seed justification — fabro-meta-c9f2 is closed and its fix's ceiling is now measured insufficient; no open seed tracks the residual.

**4. Add an acknowledgment/waiver mechanism to prompt-lint check 5 (debt this run created)**
- **Change:** in `.fabro/scripts/prompt-lint.nu` `routing-field-schema-warnings`, support an ack (e.g. an `x-routing-intended` marker property in the schema, or a waiver list at the top of the script) so acknowledged schemas stay silent.
- **Grounding:** this run's own diff added check 5, which now emits **5 permanent true-positive warnings every invocation** (all five routing-named fields on `develop/schemas/planner-output.schema.json`, which intentionally routes) plus the pre-existing date pin — 6 warnings per lint run, forever. The reviewer journal itself had to adjudicate them: *"the lint arm's 5 new warnings are disclosed true positives … gate unaffected."*
- **Expected effect:** the warning stays actionable signal; a future genuinely-unintended routing-named field doesn't drown in acknowledged noise.
- **Seed:** new-seed justification — fabro-a211 (closed by this run) introduced the warning-only design; no open seed covers the ack mechanism.

**5. Ship the already-seeded `.codex/instructions.md` ERROR-noise fix — this run re-proves it**
- **Change:** none to design — implement open **fabro-5c45** / **fabro-a701** (demote expected-absent optional-file reads from ERROR to debug/info in the session-init path). Consider folding in the 26 `unsupported_control` WARN pairs (provider protocol can't carry the tool-result error flag — fired on each of the implementer's 2 failed shell calls) into the same "noise drowns signal" pass.
- **Grounding:** worker log shows 6 ERROR lines this run (2 per agent session × 3 sessions) for the absent `/workspace/fabro/.codex/instructions.md`, plus the 26 protocol warnings — 32 warn+ lines total, **zero actionable** (from `fabro_run_logs`).
- **Expected effect:** error channel becomes trustworthy again for line-watch/rootprint sweeps; zero new design work.
- **Seed:** fabro-5c45 (engine, open) and fabro-a701 (open) — already covered; this run is fresh recurrence evidence.

**6. Kill the per-lap `context_update_dropped: output.planner` WARN**
- **Change:** one line in `.fabro/workflows/develop/workflow.fabro` planner node: add `output.planner` to `context_allow_keys` (or suppress the drop notice when the routing-active merge succeeded).
- **Grounding:** worker log 04:03:15 — `Run notice code="context_update_dropped" … dropped: output.planner`, fired on an otherwise perfectly-behaved planner pass (the routing-active merge from fabro-0a4c delivered all context keys; the residual `output.planner` payload is redundant).
- **Expected effect:** one less WARN per planner visit; same output-key hygiene family as open **fabro-7028** (gatebounce twin) — fold into its fix.

---

**Not recommended for change (evidence against):** the deterministic-node design performed exactly as intended — `claim_check` 85ms, gate 6s on a docs-only seed ("no crates touched" fast path), evidence 11.9KB inline under the 48KB budget (no blob detour), closeout 385ms, and all three roles honored the journal/lesson contracts (planner observations, implementer `mx-dd7b40`, reviewer adjudication). The 2 implementer shell errors were nu gotchas already captured as a durable lesson; no workflow change warranted.

**Sources:** run events/checkpoints and stage transcripts (fabro_run_get/events), stage journal `.fabro/journal/01M2VX6PVR87WS6XJQNY6YMMFE.jsonl`, worker tracing log (fabro_run_logs), seed tracker `.seeds/issues.jsonl`, and this run's own diff to `.fabro/scripts/prompt-lint.nu` / docs files. Could not inspect from here: the actual state of PR #243 or the terminal status of run 01M2TWKB0M27E37BXX6ZDP0X8E (recommendation 1's resolution step would settle it).
