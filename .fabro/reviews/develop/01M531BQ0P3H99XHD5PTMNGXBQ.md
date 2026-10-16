# Improve review — run 01M531BQ0P3H99XHD5PTMNGXBQ

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (288.7s wall, revisor pass — reason and cost in run detail)
- generated: 2026-10-16 19:09+0000 by revisor `fabro_ask`

---

# Recommendations — grounded in run 01M531BQ0P3H99XHD5PTMNGXBQ (Loop lane, seed fabro-4cc3)

Run baseline (from run events/conclusion): 288.7s wall, $0.3212 total, planner = 40% of cost across 2 visits, implementer 42%, gate green and review approved first-pass, 1 deterministic bounce (claim_check), PR #413. Ordered by expected impact:

---

**1. Make preflight's anchor flags agree with claim-check's — the bounce lap was avoidable.**
What happened (from run events): preflight flagged `merge-upstream/workflow.toml:20 [missing_file]` on fabro-4cc3 — a path that resolves fine (claim_check@2 passed it; the implementer journaled "Anchor-flag false positive") — while *missing* the genuinely stale bare `scripts/stage-journal.nu` citation that claim_check@1 correctly bounced on. Result: planner@2 re-emitted the whole corrected body and re-claimed (33.7s, $0.0448, 17.7k input tokens ≈ 12% of wall / 14% of cost) — pure rework. Preflight also falsely marked fabro-4cc3 `in_flight` under run 01M4NV2HNFF0KGWZ8BF23J375C (which holds fabro-8932/a111), forcing extra planner adjudication.
Change: in `.fabro/workflows/develop/scripts/planner-preflight.nu` + the shared seam `.fabro/workflows/develop/scripts/anchor_check.nu`, apply claim-check's exact path-rooting rules to the preflight arm and key `in_flight` on the run actually holding the seed.
Expected effect: stale citations get corrected in planner pass 1 instead of bouncing after the claim — −1 planner visit + −1 claim_check visit per stale-citation seed (this class is common; claim-check exists for it).
Seed: **fabro-e8ae** (open, @loop) owns this seam (anchor extraction truncation; its currency-sweep note says the seam "now also feeds claim-check.nu") — extend its demand to the rooting/parity + false-in_flight arms; no other seed covers preflight/claim-check agreement.

**2. The planner's prescribed `fabro_run_search` tool is structurally absent — enable it.**
What happened: the graph declares `x.fabro_tools="fabro_run_search"` on the planner node and the prompt's step 5 mandates it for in-flight exclusion, but the delivered tool list (event seq 85, `attractor.tools`) contains zero fabro tools; the planner journaled "no fabro_run_search tool available — degraded arm, single-source" in *both* visits. Root cause per the recorded 96c6 posture (tracker handoff fabro-d4e4): `[run.agent] fabro_tools` only **enables**, node-level `x.fabro_tools` only **names** — and this run/spec carries `agent.fabro_tools=false`, so the node naming is dead config. In-flight exclusion is single-sourced on the same preflight whose flags recommendation 1 just showed are unreliable — the tail risk is two runs implementing one seed (a duplicated full ~$0.32 run + wasted PR).
Change: one block in `.fabro/workflows/loop/workflow.toml`: `[run.agent] fabro_tools = true`, keeping the node-level `x.fabro_run_search` naming as least-privilege (the fabro-6f44 user-decision-(b) pattern).
Expected effect: step 5 becomes executable; the in-flight check gets its second source; the "degraded arm" journal noise disappears.
New-seed justification: fabro-6b58 bounds a call that presumes the tool works and fabro-9372 is a projection field — no existing seed names the missing enable bit for the loop planner.

**3. Port the two prompt footguns that fired in this run's implementer into the loop prompts.**
What happened (implementer@1 journal): (a) it ran `rg -rn` and the `-r n` replace flag mangled output ("workflows/n.nu"), nearly derailing the anchor verification; (b) its journal observation leaked non-English text ("…差点 misled…"), which flows into later stages via the `journal` context key.
Change: two one-liners in `.fabro/workflows/loop/prompts/implementer.md`: "never `rg -rn` — `-r` replaces, use `-n`" and "Always respond in English."
Expected effect: no re-derived verification calls; clean journal context handed to reviewer/planner.
Seeds: **fabro-6997** (open, @loop — the rg one-liner) and **fabro-eaa0** (open, @loop — English responses); both currently target develop-lane prompt files, and this run is fresh evidence the traps fire in the loop lane's separate prompts — carry the same lines when landing them.

**4. Cap the tracker-row diff context in evidence.nu — 47KB capture, most of it unrelated seeds.**
What happened: evidence@1 output was 46,881 bytes; the "seed work: complete diff" section diffs `.seeds/issues.jsonl` with -U3 context, inlining three entire unrelated multi-KB seed bodies (fabro-2357, fabro-6ac5, fabro-b46e) around the one-line row change — while the reviewer's preamble inline cap is 16KB (`x.preamble_inline_max_kb=16`), so most of it never reaches the reviewer anyway (this reviewer re-verified from the tree in 5 tool calls). The recorded sibling incident (fabro-020b basis) cost a 68.8s reviewer blob-paging detour.
Change: in `.fabro/workflows/develop/scripts/evidence.nu`, render the tracker file with `-U0` (or filter context hunks to rows naming the current seed id); keep full diffs for real code files.
Expected effect: captures drop from ~47KB to a few KB; reviewer preamble stays inline, no blob detours.
Seed: **fabro-020b** (open, @loop) caps the *loop-churn* section only ("keep full diffs for seed-work files") — the bloat here came from the seed-work section's tracker context. New-seed justification: no existing seed covers tracker-row diff context in the seed-work section (extend fabro-020b's demand or file a sibling).

**5. Loop-gate runs lint-nu + prompt-lint twice.**
What happened: tester@1 (19.2s, 12,292-byte output) — tier 1 (`just validate-workflows`) already ran lint-nu (64 scripts) + prompt-lint (77 files); tier 2 (`battery-runner all`) re-ran both verbatim — the entire note/warn block appears twice in the gate output, which is also what feeds the implementer's preamble on any gate-red loop.
Change: in `.fabro/workflows/loop/scripts/loop-gate.nu`, have one tier own the lints (e.g. battery-runner batteries-only, since validate-workflows already lints) — or pass the product gate's shared surface unchanged and drop the duplicate call here.
Expected effect: green-gate wall ~19s → ~10–12s; halved gate output in every red-loop cycle.
New-seed justification: fabro-eae5 (landed) deliberately bundled lints+batteries for the product gate's single-call shape — no seed covers the loop gate's double execution between tiers 1 and 2.

**6. Stop pouring the 65-row ready firehose into the planner.**
What happened: planner@1's opening `seeds ready --assignee loop --limit 200` returned 65 rows / ~9.4KB (events seq 92–95) just to learn the queue top — preflight had already narrowed candidates to five. Planner is the run's cost center ($0.128 across two visits, 40% of total).
Change: `.fabro/workflows/loop/prompts/planner.md` step 1: use the priority-flagged/top view first (`--priority high` per the sweep note; `--first` doesn't exist), full listing only when top candidates are unclaimable — exactly today's shape (2 in-flight skips, then fabro-4cc3).
Expected effect: several KB less planner context per pass; fewer wasted adjudications below the queue top.
Seed: **fabro-c3b4** (open, @loop; its 2026-10-11 rewrite targets develop's planner + bounding `fabro_runs_list`) — extend to the loop planner prompt when landing.

**7. Superseded-close fabro-c0c2 and land the automated sibling sweep.**
What happened: fabro-c0c2 is an open, unassigned duplicate of exactly the rewire+delete this run landed (fabro-4cc3, PR #413). The planner adjudicated the overlap in *both* passes and the implementer journaled it a third time — and the next planner will pay it again.
Change: next loop planner run performs the superseded-close (`seeds close fabro-c0c2 --reason "superseded: fix landed in <PR-413 sha>"`); durably, land the closeout sweep so merged diffs auto-mark satisfied siblings.
Expected effect: one fewer re-adjudication per planner pass; the queue stays truthful after merges.
Seed: **fabro-ab38** (open, @loop) is exactly the automated sweep; the one-off close needs no seed (the planner's sanctioned superseded-close form).

**8. Mid-run heartbeat for long agent stages.**
What happened: implementer ran 103.6s and planner@1 70.6s with zero outward signal — notifications fire only on `run.completed`/`run.failed` (workflow.toml), so ~3 of the run's 4.8 minutes are silent to #dev-fabro.
Change: land the long-stage heartbeat notification.
Seed: **fabro-b769** (open, @loop). Expected effect: live visibility during agent silences; no cost change.

---

Sources: stage transcripts, gate outputs, timings and cost from `fabro_run_events`/run projection; seed bodies and statuses from workspace file `.seeds/issues.jsonl`; graph/workflow config from `.fabro/workflows/loop/workflow.fabro` and `workflow.toml`; gate structure from `.fabro/workflows/loop/scripts/loop-gate.nu`. Not inspected: the internal line-by-line logic of `planner-preflight.nu`/`anchor_check.nu` (recommendation 1's exact code site — the defect is characterized from this run's observed flags and the implementer's journal, not from reading the script).
