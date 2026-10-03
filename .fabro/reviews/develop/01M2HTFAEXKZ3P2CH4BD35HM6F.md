# Improve review — run 01M2HTFAEXKZ3P2CH4BD35HM6F

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (3.9 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-15 06:10+0000 by revisor `fabro_ask`

---

All facts below are from this run's events (seq numbers), checkpoints, and worker logs. Run profile: 224.6 s wall, $0.2325 total — implementer 115.4 s / $0.119 (51%), planner 60.2 s / $0.075 (32%), reviewer 30.7 s / $0.039 (17%); all deterministic scripts together 4.0 s. First-pass green everywhere, zero bounces — so the levers are turn-count and context hygiene, not cycle design.

## Recommendations (by expected impact)

**1. Document the `ml record` flag contract in the implementer prompt — seed fabro-96bd**
- Change: `.fabro/workflows/develop/prompts/implementer.md`, step 6 — add the per-type flag table (convention requires `--content` in addition to `--description`).
- Evidence: implementer's first `ml record` failed with "convention records are missing required flag(s): --content" (seq 123–125), retried successfully (seq 129–131) — one wasted call + LLM turn on the run's costliest stage.
- Effect: removes the guaranteed first-attempt failure on every lesson-capturing pass.

**2. Make `ml record` print the mx-id and document a recovery line — seed fabro-8d81**
- Change: upstream `ml` prints the mx-id on success; add the `ml search` recovery one-liner to implementer.md step 6.
- Evidence: after recording, the implementer burned **four** calls hunting the id — `ml status` (no id), `grep -rl` (no match, wording differed), `ls .mulch`, finally a python3 JSON parse (seq 135–155). That's 6 of 13 implementer tool calls on the ml-record cluster overall (~40–50 s ≈ 20% of run wall).
- Effect: mx-id available in one call; kills the entire hunt class.

**3. Raise `preamble_budget_kb` 24 → 32 — seed fabro-8d2c**
- Change: graph attr in `workflow.fabro` (Develop graph header).
- Evidence: a **9.4 KB** evidence capture was still demoted to a blob ref (seq 185 marker), forcing the reviewer's mandatory blob-read detour (seq 195) — even though it sits under the reviewer's 16 KB `preamble_inline_max_kb` ceiling; the aggregate 24 KB budget is what fired. This is the same friction that already forced the 12→24 bump (graph comment).
- Effect: typical captures (<~20 KB) render inline; one fewer tool round-trip and one less demote-painpoint per review.

**4. Top-N `sd ready` view in the planner — seed fabro-c3b4**
- Change: planner prompt / PROJECT_FACTS sd table — first call becomes a bounded top-N view.
- Evidence: planner's opening `sd ready --assignee fabro --limit 200` returned 200 rows / **27,874 bytes** with `stdout_truncated: true` (seq 29–31); only the first row was ever used (fabro-3f62, picked at seq 36). ~7k tokens of dead context in the run's second-costliest stage.
- Effect: smaller first turn, cheaper/faster planning (planner = 60 s of a 3.7-min run for a routine claim).

**5. Stop `ml record` from mutating `.mulch/mulch.config.yaml` mid-run — seed fabro-b94d**
- Change: `ml` domain auto-creation should not rewrite the tracked config during a pass (pre-register the domain, or write config post-run).
- Evidence: the record auto-created domain "develop-workflow" and edited `.mulch/mulch.config.yaml` (implementer diff, seq 165); the reviewer then had to adjudicate 4 churn files instead of 2 (seq 200). On a code seed this becomes the "changed files NOT named by the seed spec" anomaly section — mandatory per-file adjudication.
- Effect: review scope shrinks to the deliverable; fewer churn adjudications and less rejection risk.

**6. Give the painpoint mandate teeth in the routing schema — seed fabro-017f** (companion: fabro-50de)
- Change: routing `output_schema` for implementer/reviewer — require a non-empty painpoints array or explicit `[]` justification.
- Evidence: the reviewer performed the blob-read workaround yet reported `painpoints: []` (seq 200), against its own "a workaround is a painpoint" clause; the implementer likewise filed the ml-record failure under observations (seq 158). The friction that drives fixes like #3 never reaches the journal channel that historically produced them.
- Effect: real friction flows to the improve loop instead of dying as observations.

**7. Gate the implementer's `sd show` re-fetch on brief quality — seed fabro-4881**
- Change: implementer.md step 1 — re-fetch the seed only when the brief is thin.
- Evidence: implementer call #1 re-fetched fabro-3f62 verbatim (seq 81–83, 1.4 KB) although the planner had read the same seed 2 min earlier (seq 37–39) and the brief already carried the distilled bullets; the evidence capture later embeds the authoritative spec a third time.
- Effect: ~1 call + 1 LLM turn (~7–10 s) saved per pass.

**8. Downgrade the by-design preamble allow-key warn — seed fabro-8275**
- Change: `fabro_workflow::lifecycle::fidelity` log level for expected-absent keys → info.
- Evidence: worker log's only WARN this run is `preamble_allow_keys entry absent ... key=output.gate_known_bug_hits` (06:02:18) — by design on first visits/green runs per the gatebounce node's own comment.
- Effect: warn+ log view carries only actionable signals.

**9. Silence the expected `.codex/instructions.md` missing-file error — new seed**
- Change: engine agent-session memory init (the `coding_session_initialize` read path) — treat an absent optional `/workspace/fabro/.codex/instructions.md` as info, not ERROR.
- Evidence: 6 of the 7 warn+ worker-log lines this run are exactly this ERROR, twice per agent session across all three sessions.
- New-seed justification: no seed in this run's 200-row ready listing covers agent-session memory-init diagnostics or `.codex` instructions noise.
- Effect: error-level logs become trustworthy for triage.

**10. Quote anchor text instead of line numbers in briefs — seed fabro-c0ca**
- Change: planner.md step 6 — insertion points as quoted anchor strings, not `line 27`/`line 30`.
- Evidence: this run's brief and PASS report both cite absolute line numbers (planner seq 60, implementer seq 158); correct only because the tree was fresh — any re-plan after a changes cycle shifts them.
- Effect: briefs survive re-plan drift; implementer probes stop depending on stale line math.

Not recommended for change based on this run: graph topology (single-pass, no gate/review cycles — the gatebounce/evidence/closeout nodes never misfired), reasoning_effort settings (planner 590 / implementer 879 / reviewer 314 reasoning tokens are already lean), and the tester timeout (gate ran 3.5 s against a 20 m ceiling on a no-crates-touched seed; the cold-15 min worst case the timeout documents was not exercised).
