# Improve review — run 01M2AQY97EG1YDTSQES8FFFAS2

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (16.0 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-12 12:23+0000 by revisor `fabro_ask`

---

All evidence below is from run `01M2AQY97EG1YDTSQES8FFFAS2` events/checkpoints (seed `fabro-7e8d`, a 17-line `.config/nextest.toml` change), the stage journals, and the worker log. Run totals: 848.7s wall, $0.260 LLM cost; stage split — planner 68s/$0.159, implementer 715s/$0.066 (of which 655.8s tool time), tester 4.8s, reviewer 30.6s/$0.035.

## Recommendations, by expected impact

**1. Bound the planner's in-flight check with `created_since` — it pulled all 79 develop runs into context.**
- What happened: planner called `fabro_runs_list` with no window (event seq 37/40); the 79-run JSON grew its conversation from ~12.6k to ~34.3k tokens (seq 35 → 43 context breakdowns). The planner ended up 61% of total run cost ($0.159 of $0.260) and 39% of run inference time.
- Change: in `.fabro/workflows/develop/prompts/planner.md`, step 4 (IN-FLIGHT PR CHECK), mandate `created_since` (~24–48h — anything older is merged/closed by definition, as this run's own list confirms) plus explicit self-exclusion (open seed fabro-6b58 is exactly this).
- Effect: planner input drops ~20k tokens (~$0.07/run, roughly halving planner cost) and ~15–20s wall, on every develop run.

**2. Make the brief's verification bullets match the diff's file types — this config-only seed was prescribed compile-level checks.**
- What happened: the planner's brief (criterion 4, checkpoint seq 87) demanded `cargo nextest list`, a focused `nextest run`, fmt, and clippy for a TOML-only diff. Result: 655.8s of implementer tool time — 82% of the entire run wall — for 17 lines of config. The tester gate itself took 4.8s ("no crates touched"). The implementer journal records the first `cargo nextest list` timing out at 300s on the cold build, retry finishing in ~3.6 min.
- Change: in `.fabro/workflows/develop/prompts/planner.md` step 6 (cheapest-first), add: for nextest/config-only seeds, the parse-level check is `cargo nextest show-config` (no compilation) — not `cargo nextest list` (which builds all test targets) — and drop fmt/clippy bullets when no Rust file is in the diff.
- Effect: implementer tool time on config-only seeds falls from ~11 min to ~2–3 min (the one focused test still needs a compile); run wall roughly halves.

**3. Extend the 600s-timeout-rule to `cargo nextest list/run`.**
- What happened: the first `cargo nextest list` died at a 300s default timeout, then the doubled-timeout retry burned another ~3.6 min (implementer journal, seq 91–153 window). The implementer prompt's "timeout_ms >= 600000" rule covers `cargo build`/`cargo run` but not nextest, so the model used the default.
- Change: one sentence in `.fabro/workflows/develop/prompts/implementer.md` step 4's cost-tier paragraph: the 600000ms minimum applies to `cargo nextest list`/`run` too.
- Effect: no timeout+retry round trip on cold containers — saves ~5 min of wall and one wasted LLM turn whenever a cold compile is unavoidable (i.e., whenever recommendation 2 doesn't apply).

**4. Stop the `sd ready` firehose and the truncation blindness it caused.**
- What happened: `sd ready --assignee fabro --limit 200` returned 200 seeds, 27,996 bytes, `stdout_truncated: true` (seq 31) — yet the planner journal reported `painpoints: []` (seq 83). The improve loop went blind at the exact moment friction occurred; this is open seed fabro-ac16 happening again, live. The planner only needed the priority-ordered head to pick `fabro-7e8d`.
- Change: in `.fabro/workflows/develop/prompts/project-facts.md` (sd command table), split the limit rule: claim path uses `--limit 20` (output is priority-ordered; the head is authoritative); keep `--limit 200` only for tracker-empty verification.
- Effect: ~7k fewer planner context tokens and no truncated output on the happy path; the truncation-painpoint gap narrows to the rare empty-tracker case.

**5. Default PR prose generation to non-strict JSON.**
- What happened: worker log 12:18:18 — "PR content structured generation failed; retrying once without strict JSON output" (PR #133). The guaranteed-to-fail strict first pass plus rescue retry wasted one LLM call and ~10–15s at run close. Closed seed fabro-2b7a documented this as duplicate of open fabro-41b1; it recurred here anyway.
- Change: in `lib/components/fabro-workflow/src/pipeline/pull_request.rs` (~line 452), make the non-strict path the first and only attempt for PR title/body prose.
- Effect: exactly one LLM call for PR content on the happy path; ~10–15s and one call saved per run.

**6. Enforce one edit per file per tool batch.**
- What happened: worker log 12:04:11 — "concurrent write to the same file in one batch; serializing path=/workspace/fabro/.config/nextest.toml". The implementer issued the two `[[profile.*.overrides]]` insertions as parallel `edit_file` calls; serialization saved it this time, but the swallowed-write near-miss class is documented in the prompt itself (19 warnings in a prior run).
- Change: either a hard rule line in `.fabro/workflows/develop/prompts/implementer.md` step 2, or rejection in `lib/components/fabro-agent/src/write_locks.rs` (return an error instead of silently serializing same-path batch writes) — open seeds fabro-1d94/fabro-4601.
- Effect: eliminates the silent-lost-hunk failure mode on multi-hunk edits; the engine path additionally converts a WARN into a deterministic failure the model can immediately correct.

**7. Downgrade expected-absent `preamble_allow_keys` to info.**
- What happened: worker log 12:03:50 — "preamble_allow_keys entry absent from context node=implementer key=output.gate_known_bug_hits". That key exists only after a gate-red bounce (fabro-56f4 design); on a first-visit green run like this one its absence is by construction, yet it logged at WARN.
- Change: in `lib/components/fabro-workflow/src/lifecycle/fidelity.rs`, treat keys declared by conditional producer nodes (gatebounce) as expected-absent at info level (open seed fabro-8275).
- Effect: the warn channel carries only real drift; triage of worker logs stops tripping on every clean run.

**8. Quiet the `.codex/instructions.md` probe.**
- What happened: six ERROR-level lines this run (12:02:28, 12:03:50, 12:17:24 — twice per agent stage) from the docker fs driver reading `/workspace/fabro/.codex/instructions.md`, a file this repo doesn't have. The fabro-agent README documents it as optional provider-specific discovery.
- Change: in the project-doc discovery path of `lib/components/fabro-agent` (per README: AGENTS/CLAUDE/GEMINI/`.codex` discovery), probe existence before read and log a miss at debug, not error.
- Effect: 6 fewer error lines per 3-agent run — error logs become trustworthy signal, which matters the day a real fs failure needs spotting.

Not recommended for action from this run: the evidence pipe (4.8KB capture delivered inline, no blob detour — fabro-020b's cap wasn't needed here), the reviewer (30.6s, $0.035, independent verification via grep — working as designed), and closeout (0.4s deterministic). The one UX-adjacent observation I couldn't verify from workspace files is whether `cargo nextest show-config` is available at the pinned nextest version in `fabro-toolchain:noble` — worth a 10-second check before landing recommendation 2.
