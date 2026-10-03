# Improve review — run 01M1RGQHJ5RV8Y28M2GH97XGEZ

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (3.7 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-05 10:19+0000 by revisor `fabro_ask`

---

Grounded in this run's events, journals, timings, and worker log — ranked by expected impact. Run totals for scale: 189.9 s wall, $0.149 LLM cost, all 6 stages first-pass green (planner alone was $0.084 / 56% of cost, 97.5 s inference).

---

**1. Fix evidence.nu's seed-work classification for platform-targeting seeds** — *node: `evidence` (script `.fabro/workflows/develop/scripts/evidence.nu`)*
What happened: the capture header read `seed-work=0 files ... loop-churn=3 files +15/-5` — the seed's entire deliverable (the two prompt files, +14/−4) was filed under "loop churn," so the "complete seed-work diff" section the reviewer is told to judge from was empty (from the evidence stage output, seq 161). The reviewer had to reconstruct verification with its own shell calls (seq 180–182) and its journal flags exactly this.
Change: when seed-work=0 and the churn files match paths named in the seed spec/brief, classify them as seed work (or emit the churn diff). This is seed **fabro-4b57**, already open.
Effect: reviewer approves from the capture instead of a shell detour, and platform seeds stop risking a `Verification blocked` re-capture cycle.

**2. Pre-trust the repo mise config in run containers** — *engine sandbox init / `fabro-toolchain:noble` image*
What happened: the planner's first two shell calls (`sd ready`, `ml prime`, seq 32–37) both failed with `mise ERROR ... .mise.toml are not trusted`, wasting two tool calls and one LLM turn before it ran `mise trust` itself (seq 41).
Change: run `mise trust` at sandbox provisioning. This is seed **fabro-66db**, already filed.
Effect: removes a deterministic 1-turn + 2-call tax from the most expensive stage of *every* run.

**3. Fast-path `sd show` when the goal names a seed id** — *`.fabro/workflows/develop/prompts/planner.md`, step 1*
What happened: the goal was "implement fabro-56c2: …", yet step 1's unconditional "Run `sd ready --limit 200`" pulled a ~6 KB listing of ~55 seeds (seq 42) before the `sd show fabro-56c2` (seq 47). Seed **fabro-7e88** covers this (13.9 KB in the prior run).
Change: one sentence in step 1 — if the goal contains a seed id, `sd show <id> --format json` first; fall back to `sd ready` only if it doesn't resolve.
Effect: ~6 KB less context and one fewer call in the stage that was 56% of this run's cost.

**4. Make the journal payload shape retry-proof** — *`.fabro/workflows/develop/prompts/planner.md` (and implementer/reviewer), Journal section*
What happened: the planner's first final answer emitted malformed JSON (`["observations" : "none"]`), the routing validator rejected it ("key must be a string at line 1 column 2647", seq 91), and the retry burned ~13 s and a full-context re-read (~$0.01, seq 92–94). The likely cause: `painpoints` is an array of `{text}` objects while `observations` is an array of strings — the asymmetry invites the slip.
Change: make both plain string arrays (`"painpoints": ["<text>", ...]`), or add engine-side repair of trivial syntax errors before spending a retry.
Effect: eliminates a whole retry turn per malformed emission; this one cost ~13% of the planner's wall time.

**5. Correct the disproved fs_hide claim in workflow.fabro comments** — *`.fabro/workflows/develop/workflow.fabro`, implementer node comment (~line 96–101, "hidden paths are unwritable anyway")*
What happened: this run's own planner journal names it: the comment still asserts the shell-bypassed belief fabro-56c2 just removed from the prompts. Side effect already visible: the **reviewer** — which has *no* fs_hide — believed reads were denied and avoided its file tools unnecessarily ("Verified via shell reads since fs_hide denies read_file…", reviewer journal; its reasoning at seq 179 shows the misbelief).
Change: rewrite the comment to "fs_hide binds file tools only; shell reads/writes succeed — policy governs the shell," and note per-node scope (planner/implementer hidden, reviewer not).
Effect: docs stop re-teaching the disproved model to future agents and humans; the reviewer uses its cheaper native read tools. The planner's painpoint explicitly requests a follow-up seed for this.

**6. Fix the Pipeline progress header denominator** — *engine preamble rendering*
What happened: the implementer's prompt opened "Pipeline progress: 0 of 6 stages completed" with planner already done; the reviewer saw "2 of 6" with four stages done (from both stage prompts, seq 105/170).
Change: count completed nodes, not per-thread visits (seed **fabro-a0e3**).
Effect: every agent stage gets correct loop-state information instead of stale-zero.

**7. Stop the stage-journal hook from erroring on no-data nodes** — *`.fabro/workflows/develop/scripts/stage-journal.nu`*
What happened: the only warn in the worker log is `Non-blocking hook failed … hook=stage-journal … exited with code 1` at 10:10:15 — the `start` node completion, which has no journal payload.
Change: exit 0 (emit nothing) for `start`/`exit` completions.
Effect: clean warn channel, so a *real* hook failure isn't masked by routine noise.

**8. Make the implementer's step-1 spec re-fetch conditional** — *`.fabro/workflows/develop/prompts/implementer.md`, step 1*
What happened: step 1 orders "Re-read the seed requirements from `sd show`" unconditionally; the implementer silently skipped it this run (brief was complete) and succeeded — the instruction is dead weight that a compliant model would pay for (seed **fabro-a67f**).
Change: "Re-read via `sd show` only if the brief is thin or ambiguous."
Effect: one fewer call + smaller context on every well-briefed pass, without losing the fallback.

**9. Asynchronize checkpoint metadata snapshots** — *engine checkpoint path*
What happened: eight blocking metadata snapshots (~1.7–2.0 s each, seq 16/100/145/155/165/192/202/211 ≈ 14 s total) sat serially between stages — 4 s gap planner→implementer, 4 s implementer→tester, 7 s reviewer→closeout. Seed **fabro-cf03** is filed.
Effect: ~5–7% wall-time reduction per run with no semantic change.

---

One positive worth keeping: the fresh brief text ("both edits go through the shell yourself…") let the implementer do the whole job in 5 shell calls / 37 s with zero fs_hide discovery — direct proof that brief-level guidance eliminates the waste the planner paid for (rec 5 closes the last place the old claim lives). Evidence sources: run events (seq cited), stage journals (`planner@1`, `implementer@1`, `reviewer@1`), evidence capture blob, and the worker warn log. I could not inspect blob contents (tester/evidence outputs) directly beyond their previews, and engine internals (preamble renderer, checkpoint loop) are referenced only via their observed outputs here.
