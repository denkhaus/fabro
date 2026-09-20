# Improve review — run 01M1Z1PK3D7R9MEWDP7QDZMA1D

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (2.6 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-07 23:09+0000 by revisor `fabro_ask`

---

All evidence below is from this run's events (seq numbers), stage timings, billing, and the journal — run 01M1Z1PK3D7R9MEWDP7QDZMA1D, seed fabro-cf76 (a one-line edit to `.fabro/workflows/develop/prompts/planner.md`), 2m19s wall, $0.120, 186.8k tokens, 0 retries.

## Recommendations, ordered by expected impact

**1. Delete the `gh`-based in-flight PR check from planner step 4 — it fails every run now.**
- Change: `.fabro/workflows/develop/prompts/planner.md`, step 4 (IN-FLIGHT PR CHECK): remove the `gh pr list --state open` mandate and the gh fail-open clause; point at the credential-less runs-list source that fabro-06e0 (user decision D2) already decreed ("planner prompt no longer mentions gh").
- Evidence: events seq 33–35 — `gh pr list` → exit 127, `gh: command not found` (gh was deliberately removed from the image); the planner journaled this exact painpoint. The double-pick guard (fabro-22e4) was silently disabled while this run opened PR #61.
- Effect: kills one guaranteed-failing tool call + one wasted LLM turn per planner pass (~5 s), and restores an actual safety control instead of a fail-open no-op.

**2. Enforce the journal shape in the routing output schema — the planner shipped corrupted JSON this run.**
- Change: `workflow.fabro`, planner/implementer/reviewer nodes: extend `output_schema` beyond "routing" to require `context_updates.journal.{painpoints[],observations[]}` (open seeds fabro-017f / fabro-270c already name this).
- Evidence: event seq 59 — the planner's final message contains a malformed first JSON (`"journal": {...}, ["none"][1]`), then "Wait — the journal object above got malformed. Corrected final JSON:" plus a complete second JSON blob. Response doubled to 4,254 chars; the parser had to survive it. With `output_retries=2`, a slightly different corruption costs a full ~35 s planner retry.
- Effect: single-parse final JSON on every agent pass; removes the retry cliff and ~1k wasted output tokens per occurrence.

**3. Stop feeding the planner the 168-seed firehose.**
- Change: `.fabro/workflows/develop/prompts/planner.md`, sd command table: replace `sd ready --assignee fabro --limit 200` as the *first* call with a top-N view (e.g. `--limit 15`), keeping `--limit 200` only for the park/empty checks (open seed fabro-c3b4).
- Evidence: event seq 36 — `sd ready --limit 200` returned 168 issues, 22.5 KB, `stdout_truncated: true`; the planner only used line 1 (fabro-cf76). Those 22.5 KB rode along in every one of the planner's 5 subsequent turns (conversation grew 3.2k→10.9k tokens), making it the costliest stage: 36.4 s / $0.052 = 43% of run cost for picking one seed.
- Effect: ~20 KB less context per planner pass → fewer/cheaper turns; planner wall time should drop by roughly a third on trivial claims.

**4. Path-scope the tester gate for prompt-only seeds.**
- Change: `tester` node in `workflow.fabro` (or the `qualitygate` recipe): when the per-seed diff touches no `lib/`/`apps/` source, run the fast lane (fmt-check + tracker lint) instead of the full gate (open seed fabro-01b9).
- Evidence: tester ran `just qualitygate` for 8.9 s on a tree whose entire seed diff was one markdown line; the gate's own output says "no crates touched … format clean" (tester output blob, rendered in the reviewer preamble). The graph comment budgets 20 m for a cold touched-crates gate — that's the real exposure this avoids.
- Effect: ~9 s saved now per prompt-only seed; minutes saved whenever such a run lands on a cold cache.

**5. Cut the ~19% wall-time tax of per-stage checkpoint/metadata snapshots.**
- Change: engine — make checkpoint metadata snapshots async or branch-point-only (open seed fabro-cf03); on trivial runs, coalesce the snapshot+commit+push between back-to-back command nodes.
- Evidence: inter-stage gaps this run: implementer→tester 6.0 s, tester→evidence 9.3 s (snapshot alone 5.1 s), evidence→reviewer 5.3 s, reviewer→closeout 5.3 s — ≈26 s of the 139 s wall, for 7 snapshots growing 92 KB→355 KB with zero LLM work in between.
- Effect: ~20% faster runs end-to-end at identical safety (snapshots still land, just off the critical path).

**6. Make the brief actually bulleted — it violated its own format contract.**
- Change: `.fabro/workflows/develop/prompts/planner.md`, step 6: require newline-separated bullets inside the `current_seed_brief` JSON string (`\n- `), not one mashed line.
- Evidence: the emitted `current_seed_brief` (context values, seq 62) crams all criteria into a single line ("Acceptance criteria: - New rule… - When… - Keep…"), despite step 6 demanding BULLETED criteria; the implementer (seq 107) had to re-split six criteria itself for the PASS/FAIL report — exactly the misparse surface the bullets were meant to remove.
- Effect: cleaner criterion→PASS/FAIL mapping for implementer and reviewer; fewer ambiguity-driven Changes-requested cycles (~2.5 min + ~$0.15 each, per the seed's own estimate).

**7. Share the prompt-cache prefix across stages.**
- Change: engine — reuse a cache prefix for the stable system+tools+memory block (open seed fabro-944d; `cache_write_tokens=0` throughout).
- Evidence: billing shows identical ~8.5k-token stable prefix (system ~1.1k, tools ~1.6k, memory ~5.8k) re-priced as fresh input at the first turn of each of the 3 agent stages (e.g. planner turn 1: 11,798 input tokens, $0.017); 134.7k tokens were cache *reads* but nothing was ever *written*.
- Effect: roughly 20–25% input-cost reduction on multi-stage runs at zero behavioral change.

**8. Minor: downgrade the by-design `gate_known_bug_hits` warning.**
- Change: engine fidelity lint — the warn "preamble_allow_keys entry absent … key=output.gate_known_bug_hits" (run log, the only warn in the worker log) fires on every first-visit implementer because that key only exists after a gate-red bounce. Open seed fabro-8275 covers it.
- Effect: a clean warn log, so real degradations (like the disabled gh guard in #1) aren't masked by expected noise.

Not inspected: the PR postlude (auto-merge/PR-body model call) ran after `run.completed` (PR #61 at seq 177, ~11 s) — I saw no failure evidence for it in this run's events, so I made no recommendation there despite open seed fabro-6a5a.
