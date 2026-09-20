# Improve review — run 01M2HZP8ZM99KNJ4Y424SDR1FB

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (2.3 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-15 07:41+0000 by revisor `fabro_ask`

---

All evidence gathered. Every finding below maps to an existing open seed (verified against `.seeds/issues.jsonl`) — no new seeds needed; this run adds fresh grounding for each.

## Run baseline (what the recommendations are measured against)

From the run conclusion and stage events: happy-path single visit per node, zero retries, gate green in 3.6s. Total 2m04s active / **$0.143** / 231k tokens. But **100.7s of the 107.2s active time was LLM inference; tools took 6.5s total** — planner 47.2s/$0.062 (44% of cost), implementer 37.3s/$0.049, reviewer 19.3s/$0.032. The leverage in this run is **LLM turn count and context size**, not tool speed. Graph topology itself is sound on this evidence (no cycles, capture 3.2KB inline with no blob detour, stdin-scoped closeout) — so the highest-impact changes are turn/context economy, not rewiring.

## Recommendations, by expected impact

**1. Batch the planner's reconnaissance into one shell call — seed fabro-55a7.**
From run events: the planner made 5 sequential single-purpose turns (seq 29 `sd ready`, 32 `fabro_runs_list`, 37 `sd show`, 43 grep/sed basis probe, 49 claim), each with 5.8–12.2s time-to-first-token — 47.2s wall to claim a 5-line seed. Change `.fabro/workflows/develop/prompts/planner.md`: one batched recon call (`sd ready` + `sd show` + basis probe) before claiming. Effect: −2–3 LLM turns ≈ −15–20s and ~$0.02–0.03 per run, plus a shorter claim→dispatch window (narrowing the duplicate-claim race the in-flight check guards).

**2. Top-N `sd ready` view instead of the firehose — seed fabro-c3b4.**
From run events seq 30: `sd ready --limit 200` poured **27,838 bytes / 200 seed lines** into the planner's context — and it picked the first High seed on the list. Fresh evidence for this seed (prior basis was 15–24KB; now 27.8KB and growing). Change `planner.md`: top-N pick view, full listing only when top candidates are unclaimable; pass `created_since` to `fabro_runs_list` (the in-flight view here listed all history back to 2026-09-14 for no benefit). Effect: ~25KB less context per planning pass, less distraction, cheaper cache.

**3. Make the implementer's `sd show` re-fetch conditional — seed fabro-4881 (overlaps fabro-a67f).**
From run events seq 75: the implementer re-fetched `sd show fabro-3e48` although the brief already carried all 5 acceptance criteria *and the exact verification commands* — the re-fetch turn alone cost ~9s of the implementer's 36.8s inference. Change `.fabro/workflows/develop/prompts/implementer.md` step 1: reword to fetch only on thin/ambiguous/verification-only briefs (the Input section already declares the brief authoritative). Effect: −1 LLM turn ≈ −9s/~$0.02 per implementer pass, and the planner's resolved reading stays authoritative.

**4. Add the reviewer verification-economy clause — seed fabro-50c9.**
From run events: the reviewer's prompt (seq 122) already contained the complete 3,251-byte evidence capture inline — the full 5-line diff, churn counts, clean worktree — yet seq 132 re-ran the *identical* sed section extract, `grep -c`, `git diff --stat`, and `git status`; its own journal admits "rather than trusting the evidence capture alone." That verify turn cost ~11s TTFL of the reviewer's 19.3s/$0.032 (22% of run). Change `.fabro/workflows/develop/prompts/reviewer.md`: judge from the capture; tools only for claims the capture cannot show; never re-run a check whose assertion already appears in the diff. Effect: −1 turn per review ≈ −11s, and the evidence pipe finally saves the work it captures.

**5. Finish the deterministic verify dispatcher — seed fabro-6e7f (already in_progress; this run is fresh drift evidence).**
From run events: the implementer **never ran `just verify implementer`** — its prompt's "ONE mechanical verification call" — because the brief's criteria (4)/(5) rewrote verification as shell-only sed/grep plus "gate green via the deterministic tester step," and the implementer obeyed the brief over the prompt. Harmless on this Markdown-only seed (the tester correctly reported "no crates touched"), but it's exactly the advisory-rule drift this seed documents. Change: land `just verify` with the engine-injected `FABRO_STAGE` env and shrink implementer.md step 4 to one line. Effect: verification scope becomes executable and brief-proof instead of prompt-advisory.

**6. Per-node memory scoping for agent stages — seed fabro-9588.**
From run events (memory.loaded at seq 21, 67, 124): AGENTS.md, **25,271 bytes, loaded into all three agent stages** ≈ 19k tokens per run — the reviewer and planner demonstrably used none of it. The seed already proves `project_memory=false` is inert on the agent path. Change: thread memory/skills scoping through `AgentSession::initialize` (`lib/components/fabro-agent/src/session.rs`). Effect: smaller system prompts and cleaner attention on every stage of every run.

**7. Split the PROJECT_FACTS sd-command table per role — seed fabro-52b4.**
From run events: the reviewer's prompt (seq 122) carries the full 6-row tracker table including the `sd update` claim form and `sd close` exception it is forbidden to use; the implementer gets the same. Change: factor the PROJECT_FACTS include per role in `.fabro/workflows/develop/prompts/` (planner full, implementer `sd show` only, reviewer none). Effect: ~1–2KB less prompt per stage, and no forbidden-command anecdotes advertised to read-only roles.

**8. Fix the PR-postlude strict-JSON failure and the generic title — seed fabro-41b1 (extend it with the title fallback).**
From worker logs: 07:34:39 "PR content structured generation failed; retrying once without strict JSON" → 07:34:44 "retry was not JSON; salvaging prose body with deterministic title" — the guaranteed fail-then-retry path on this run too, and the salvaged title is the truncated generic goal: PR #150 "Develop the project one seed per run: claim the next open seed, impleme…". Change: make the postlude's content call non-strict (one LLM call on the happy path), and derive the deterministic fallback title from `current_seed_title` ("Implementer: add routing-consistency self-check — one route per branch"). Effect: −1 retry and −1 warn per run, and PRs become identifiable in the GitHub list.

**9. Clean the warn/error channels — seeds fabro-8275 and fabro-95c3/fabro-5c45.**
From worker logs this run: one warn `preamble_allow_keys entry absent … key=output.gate_known_bug_hits` on the implementer's green first visit (by-design absence), and six ERROR lines for the expected-absent `.codex/instructions.md` (two per agent stage). Change: fidelity module emits info for first-visit key absence; memory loader pre-probes existence and logs NotFound at debug. Effect: ~7 false alarm lines removed per 3-agent run; the error channel stays trustworthy for real failures.

**What I deliberately did not recommend:** no graph-design changes — on this run's evidence the topology (one-seed-per-run exit, structural cycle guards, gatebounce pre-step, stdin-scoped closeout) did exactly what it should, and every failure mode it guards against stayed unexercised. The two tool-execution stars of the run (tester 3.6s, evidence 0.27s) need nothing.
