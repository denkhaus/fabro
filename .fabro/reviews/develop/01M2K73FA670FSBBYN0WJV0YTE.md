# Improve review — run 01M2K73FA670FSBBYN0WJV0YTE

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (2.9 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-15 19:08+0000 by revisor `fabro_ask`

---

All evidence below is from this run's events (`fabro_run_events`), the run conclusion/checkpoints (`fabro_run_get`), and the tracker at `/workspace/fabro/.seeds/issues.jsonl`. Run shape: seed **fabro-b073** (Markdown-only prompt edit), clean single pass, gate green in 3.6 s ("no crates touched"), 143 s active, **$0.248 total** (planner $0.077 / implementer $0.142 / reviewer $0.028). What actually hurt is concentrated in tool-call economics, not graph control flow — every stage routed first-try, zero retries.

## Recommendations (by expected impact)

**1. Kill the `ml record` triple-penalty: failed call, config mutation, mx-id scavenger hunt.**
What happened: the implementer's lesson capture failed at seq 108–110 (`decision records are missing required flag(s): --title, --rationale` — the prompt's own example omits them); the **failed** call still printed `✓ Auto-created domain "loop"` and mutated `.mulch/mulch.config.yaml` (churn the reviewer then had to adjudicate, per its journal observation); the retry succeeded but printed no mx-id, so the implementer burned **6 more shell calls + LLM turns** (seq 122–159: `ml search`, `ml list`, `ml --help`, `find .mulch`, `grep mx-`, `tail|python3`) to recover `mx-74efa6`. That tail is ~34 s and ~$0.07 — half the implementer stage.
Change: `.fabro/workflows/develop/prompts/implementer.md` step 6 — document the per-type flag contract and the one-line mx-id recovery command; pre-create the `loop` domain (toolchain image or prompt) so `ml record` stops editing the tracked config.
Expected effect: ~$0.07 and ~35 s saved per run; no surprise churn for the reviewer.
Seeds: **fabro-96bd** (flag contract), **fabro-8d81** (print mx-id upstream + documented recovery line), **fabro-b94d** (stop mid-run config mutation) — all open, all three hit in this run.

**2. Bound the planner's `sd ready` firehose.** Seq 30: one `sd ready --limit 200` poured **27,783 bytes / 200 seed lines** into planner context just to learn the top of the queue; the planner is already the second-costliest stage ($0.077, 45 s for one claim).
Change: `.fabro/workflows/develop/prompts/planner.md` step 1 — top-N view (`--priority high` / first 10), full listing only when top candidates are unclaimable.
Expected effect: several KB and seconds off every planning pass.
Seed: **fabro-c3b4** (open; its "fresh evidence" line says the firehose still costs ~24 KB/pass — this run measured 27.8 KB).

**3. Fix the pipeline-progress counter.** The implementer's prompt read "0 of 7 stages completed" with the planner done (seq 71); the reviewer's read "2 of 7" with **five** nodes done (seq 189). Every stage preamble lies to the agent and the user.
Change: engine progress projection (run-level + preamble header) — count unique completed non-meta nodes.
Expected effect: honest, interpretable progress on both surfaces.
Seed: **fabro-45bf** (open, P1, explicitly covers the preamble-header surface).

**4. Make the implementer's `sd show` re-fetch conditional.** Seq 81–83: the implementer re-fetched the seed although the brief was complete and already authoritative — wasted call + turn per cycle.
Change: `.fabro/workflows/develop/prompts/implementer.md` step 1 — re-fetch only on thin/ambiguous/verification-only briefs.
Expected effect: one fewer call and LLM turn per cycle; planner's resolved reading stays authoritative.
Seed: **fabro-4881** (open).

**5. Scoped reads when the brief pins anchors.** Seq 90–92: the implementer read 15.4 KB (`sed -n 1,80p`) of `planner.md` although the brief named the exact heading and grep had already located it at line 11; the planner did the same with `sed -n '1,60p'` (13.8 KB, seq 49–51).
Change: implementer prompt shell guidance — `sed -n '<range>p'` around pinned anchors, never whole-file dumps (planner analog: heading greps only).
Expected effect: ~10–14 KB less context per platform-file pass.
Seeds: **fabro-645d** (implementer, open), **fabro-8d4c** (planner, open).

**6. New seed — carry tool errors through the zai protocol.** Seq 113/120/127/134/141/148/155: five `agent.warning unsupported_control` ("this provider protocol does not support the tool result error flag") — the failed `ml record` round-tripped without an error flag, so the model only sees failure in stdout text. Justification for a new seed: I checked the tracker; fabro-96bd covers the *prompt* contract, but no open seed covers the provider-protocol error-flag degradation itself (engine `lib/components/fabro-workflow` agent session layer). Expected effect: deterministic error signaling instead of text-sniffing, one fewer blind retry class.

**7. New seed — backfill `pull_request.state` for terminal-but-unmerged runs.** Seq 33: prior run 01M2K3NQEW is `succeeded` with PR #153 `state: null`, so the planner's in-flight guard ran in its documented degraded mode ("no exclusions") — if #153 were actually open, its seed was double-pickable. Justification: fabro-2e66 covers silent *pipeline* degradation, fabro-9372 covers `current_seed_id`, but no seed covers the null PR-state field in the `fabro_runs_list` projection. Expected effect: the in-flight guard stops running blind exactly when a PR sits unmerged.

**Keep as-is (worked this run, per reviewer journal):** the churn-only evidence capture delivered the full diff inline (4.9 KB, under the 16 KB inline cap) — the reviewer approved with **zero tool calls** in 13.8 s. Don't rebudget the evidence pipe on this evidence.

Not inspected: PR #155's post-run merge state and the engine's Rust internals beyond what events expose.
