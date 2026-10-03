# Improve review — run 01M2VFF9NRNKZVMDCDX6B9F3GX

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (2.5 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-19 00:08+0000 by revisor `fabro_ask`

---

All evidence below is from this run's events, stage timings/usage, and journals (run `01M2VFF9NRNKZVMDCDX6B9F3GX`, verification-only pass on `fabro-29f7`, approved + closed, PR #250; total 124.5 s active / $0.1897 — planner 42.2 s / $0.102, reviewer 65.9 s / $0.088). Seed IDs verified against `.seeds/issues.jsonl`.

## Recommendations, ordered by expected impact

**1. Emit per-criterion check outputs in the verification-only evidence capture — seed `fabro-f759`**
What happened: the evidence capture was 3,156 bytes of "(no seed-work files to diff)", so the reviewer re-derived every brief bullet by hand — 8 sequential shell calls (journal grep, `git log --since` drift checks, posture grep, `sd show`), each wrapped in its own ~5–10 s LLM turn. Result: 64.6 s inference / $0.088, 46% of run cost and 53% of run wall, for a seed whose checks were all mechanical.
Change: in `.fabro/workflows/develop/scripts/evidence.nu`, when seed-work is empty and the brief is verification-only, run the brief's cheap read-only checks (journal-file existence, `git log --since <gate-date> -- <paths>`, posture grep) and append their outputs as a capture section.
Expected effect: reviewer goes from 8 tool calls / ~7 LLM turns to judging from the capture in 1–2 turns — roughly −40 s and −$0.05 per verification-only run, and implementer/reviewer verification drift disappears. (`fabro-d89a` is the implementer-run sibling of the same fix.)

**2. Bound the planner's reconnaissance: top-N `sd ready` + `created_since` on `fabro_runs_list` — seeds `fabro-c3b4` (as extended 2026-09-09) and `fabro-6b58`**
What happened: `sd ready --assignee fabro --limit 200` poured 29,236 bytes / 200 seed rows into the planner conversation (`stdout_truncated: true`, event seq 38), and the `fabro_runs_list` call carried no `created_since` (arguments: `{"workflow": "develop"}` only), returning 67 runs — including archived failures from two days prior — and taking **8.58 s of the planner's 9.1 s total tool time**. The planner then burned reasoning deducing "the only non-terminal run is this run itself."
Change: `.fabro/workflows/develop/prompts/planner.md` steps 1 and 4 — top-N pick view (`sd ready --first 10` / `--priority high`, full listing only when top candidates are unclaimable), `fabro_runs_list created_since ≈ 48h`, and the one-line self-exclusion of the run's own id.
Expected effect: ~30 KB → ~2–5 KB of planner input per pass (planner input was 47,471 tokens, 54% of run cost), the 8.6 s unbounded listing drops to sub-second, and the self-run confusion class disappears. (`fabro-55a7` — batching recon into one shell call — compounds this; this run used 4 separate round-trips.)

**3. Land the reviewer per-node tool allow-list (already user-approved) — seed `fabro-269d`**
What happened: the reviewer's tool registry (event seq 81) included `edit_file`, `write_file`, `spawn_agent`, `fabro_run_create`, `fabro_run_interact` — 27 tools, 5,118 tokens of schema — while the prompt's only guarantee of read-only-ness was prose plus `fs_write=""`. The tracker records a GO decision from 2026-09-17.
Change: `reviewer` node in `.fabro/workflows/develop/workflow.fabro`: `tools="read_file,grep,glob,shell"`, minimal `fabro_tools`.
Expected effect: read-only enforcement becomes mechanical instead of policy-hoped (ADR-0019 capability-delta surface shrinks to the diff), and ~2–3 k prompt tokens per review go away. Zero design work remains — this is an adoption edit.

**4. Per-node memory/skills scoping for agent stages — seed `fabro-9588`**
What happened: both agent sessions loaded the full 25,746-byte `AGENTS.md` (planner memory: 7,484 tokens; reviewer: 6,619 tokens — from the context-window breakdowns), and the reviewer discovered two skills (rust-style-guide, improve-codebase-architecture) for a no-diff, no-Rust verification review. That's ~14 k of the run's ~72 k input tokens serving no decision.
Change: engine `AgentSession::initialize` (`lib/components/fabro-agent/src/session.rs`) — honor per-node memory/skills scoping (the attributes exist but are inert on the agent path, per the seed).
Expected effect: ~15–20% input-token reduction on tracker/no-Rust passes like this one; smaller system prompts for every tool-light stage.

**5. Scope drift-style brief criteria to behavior, not file-touch — new-seed justification: no existing seed covers brief phrasing of drift criteria (`fabro-b8ed` governs deriving per-criterion checks, `fabro-dad8` governs probe contradictions; neither addresses over-broad drift wording).**
What happened: the planner's brief bullet said "confirm no commits touching preamble-rendering engine code after the gate date" — the reviewer's own journal painpoint (stage journal, reviewer@1) flags that this "conflates file-touch with behavior drift," forcing it to adjudicate behavior-neutral lithos-llm type migrations file-by-file (events seq 94–100 show the multi-turn `git log --name-only` rabbit hole). Relatedly, the "PRs remain unmerged/**unanswered**" half of one criterion is undecidable with in-sandbox tools (gh is forbidden) — the reviewer could only verify merge-absence.
Change: `.fabro/workflows/develop/prompts/planner.md` step 7 (contradiction check): drift criteria must be phrased behavior-scoped ("rendering-behavior change," not "commits touching `<paths>`") and every check bullet must be decidable by the sandbox's tool set; annotate the chosen reading when the seed text is looser.
Expected effect: eliminates ~3 reviewer LLM rounds of churn adjudication per gate-record seed and prevents false "Changes requested" on refactor churn; undecidable bullets stop shipping.

**6. Fix the pipeline-progress header miscount — seed `fabro-9e8b`**
What happened: the reviewer's prompt opened with "Pipeline progress: 2 of 9 stages completed" (event seq 76) when four stages (start, preflight, planner, evidence) had completed and the reviewer was the fifth — a fresh recurrence of the documented post-#175 counter-evidence, now on the 9-node develop graph.
Change: the progress projection surface in `lib/` (as named by closed `fabro-45bf`) — count completed non-meta nodes, not rendered sections.
Expected effect: honest mid-run progress for humans watching the run/PR; the number is currently wrong at every intermediate snapshot.

**7. Silence the by-design `context_update_dropped: output.planner` warn — new-seed justification: `fabro-8275` covers only the read-side preamble allow-key warn; the write-side drop of the engine's own `output.<node>` dedup key is a different code path with no seed.**
What happened: event seq 61 — the planner pass ended with a warn because the engine-written `output.planner` response wrapper isn't in the planner's `context_allow_keys` (`current_seed_id,...,journal`). It fires on every green planner pass.
Change: either add `output.planner` to the planner's `context_allow_keys` in `.fabro/workflows/develop/workflow.fabro`, or downgrade the engine's drop-warn to info for engine-generated `output.*` keys.
Expected effect: warn-level run logs stop crying wolf per pass, keeping real contract regressions visible.

**Not actionable from this run:** the `fabro-c643` anchor flag (`.fabro/Dockerfile.toolchai`, truncated path) was journaled by the planner as a suspected seed-body typo; I could not confirm from the tracker whether the body or `planner-preflight.nu`'s extractor is at fault, so I'm not attaching a change to it — the existing journal observation is the correct delivery channel for now.
