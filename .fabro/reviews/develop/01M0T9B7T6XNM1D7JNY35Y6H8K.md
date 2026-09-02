# Improve review — run 01M0T9B7T6XNM1D7JNY35Y6H8K

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: failed (8.1 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-02 19:49+0000 by revisor `fabro_ask`

---

All in-graph work in this run succeeded — one seed (fabro-8208, gofib `-limit`) planned, implemented, gate-green, reviewed, closed, tracker empty in 6m57s for $0.353. The run is marked `failed(publish_failed)` anyway. Recommendations, ordered by expected impact:

**1. Fix PR-publish auth, and probe it before the run starts — not after.**
What happened: every stage succeeded, the run branch pushed fine (event seq 82, static token), then PR creation died: `Authentication failed creating pull request (403)` (worker log, 16:31:43; conclusion `reason=publish_failed`). The token can push but not open PRs, despite `integrations.github.permissions` declaring `pull_requests: write`. A second, masked defect in the same step: PR title/body generation via the LLM failed first (`Failed to parse response as JSON`, fell back to skeleton — worker log 16:31:42).
Change: grant the PAT the PR scope (or wire the declared GitHub App instead of the static token), and add a start-of-run preflight that verifies PR-creation capability (cheap API probe) plus a JSON-format guard on PR-body generation.
Effect: runs stop ending "failed" after fully successful work; a bad credential fails in ~seconds instead of after 7 minutes.

**2. Put the per-criterion verification report in ONE place, not two.**
What happened: implementer@1 was 60% of run cost ($0.213 of $0.353, 203s of 354s inference; from stage billing in run events). Its final response contains the full PASS/FAIL report **twice** — once as markdown prose, once verbatim inside the JSON `implementation_summary` (event seq ~199/206) — and both are re-read as preamble by reviewer and planner@2.
Change: `.fabro/workflows/develop/prompts/implementer.md` — state the report lives *only* in the JSON `implementation_summary`; pre-JSON text stays one short paragraph.
Effect: implementer output was 8,024 of the run's 10,693 output tokens; killing the duplicate prose cuts maybe a quarter of that, ~10–15% off total run cost and smaller preambles downstream.

**3. Replace the post-approval planner visit with a deterministic node.**
What happened: planner@2's entire job was mechanical — `sd close fabro-8208`, then `sd ready`/`sd list` → "Tracker empty" (event seq ~293–323): 14.8s inference, $0.016, and an LLM entrusted with a tracker write it could hallucinate.
Change: `.fabro/workflows/develop/workflow.fabro` — insert a command node (like `tester`) on the reviewer "Approved" edge: script closes the seed, checks `sd ready`; routes empty→exit, seeds-remain→planner. "Changes requested" still routes straight to planner (re-briefing is real LLM work).
Effect: removes one LLM visit per seed cycle (~15s, ~$0.02, compounding across seeds) and removes a misparse risk from the tracker-critical path.

**4. Append the gate output to the evidence capture.**
What happened: tester@1's output was 288 bytes yet reached context only as a blob ref; the reviewer prompt explicitly says gate output "is NOT part of the evidence capture" and points to re-running `just qualitygate`. Reviewer@1 responded by re-verifying live extensively ("verified live" ×6, re-ran tests/binary — its response, run events) — 97.5s inference, 25% of run cost.
Change: `.fabro/workflows/develop/scripts/evidence.nu` — include the gate's (tiny) stdout tail in the integrity header.
Effect: closes a documented evidence gap, removes the reviewer's main excuse for tool re-verification, and shrinks the "Verification blocked" failure class the graph already special-cases.

**5. Raise or enforce the 12 KB preamble budget.**
What happened: worker logs show the preamble budget exceeded "even after demotion" at four consecutive prompts (12,989 → 18,053 bytes vs 12,288 budget), so the blob-ref demotion machinery fired every cycle and even the 288-byte gate output got blob'd.
Change: `workflow.fabro` graph attr `preamble_budget_kb` (12 → ~20), or shrink the inputs (rec 2 does this for free).
Effect: fewer blob-ref round-trips (each costs the reviewer a tool call + turn) and no more per-cycle budget warnings.

**6. Add an `sd` cheat-sheet and call-order rule to the planner prompt.**
What happened: planner@1 wasted a call on `sd update … --format json` → `error: unknown option '--format'` (event seq 62–63), then retried; it also ran `sd ready` and `sd list` in parallel on a one-seed tracker — identical output (seq 38–43). None of this reached the painpoint channel it has for exactly this friction.
Change: `.fabro/workflows/develop/prompts/planner.md` — add exact syntax (`sd update <id> --status in_progress`, `sd close <id>`; no `--format` on writes) and "run `sd ready` first; `sd list` only if ready is empty or blockers matter."
Effect: 1–2 fewer tool calls and one fewer LLM round-trip per planning visit (~5–10s each).

**7. Stop making the implementer re-fetch the seed when the brief is complete.**
What happened: the prompt says the brief "is authoritative," then numbered step 1 orders "Re-read the seed requirements from `sd show`" — implementer@1 obeyed step 1 (event seq 94–98) despite a complete, ambiguity-annotated brief. Re-reading the raw spec risks re-importing the ambiguity the planner settled (raw seed: "Default: no limit" vs the brief's pinned 0-sentinel).
Change: `.fabro/workflows/develop/prompts/implementer.md` — `sd show` only when the brief is thin, ambiguous, or verification-only.
Effect: one fewer call/turn per cycle, and the planner's resolved reading stays authoritative.

**8. UX: publish failure ≠ work failure.**
What happened: the run summary reads `failed` / "6 of 5 stages completed" (planner counted twice) while the actual state is: gate green, review approved, seed closed, work pushed at commit `76dfe16` — recoverable by opening one PR by hand. It was archived the next day with no PR and no retry path.
Change: engine/UI — represent `publish_failed` as a distinct, recoverable terminal state with a "retry publish" action that doesn't re-run the graph; compute the progress denominator over unique nodes.
Effect: users can tell "work lost" from "PR blocked" and recover in one click.

**What already worked (no change needed):** the baked-in runner image delivered on its design — sandbox ready in 3.2s, `mise install` 136ms (vs the ~24.5s the Dockerfile header says it replaced); evidence blob-ref delivery to the reviewer worked end-to-end; the prior expertise record (mx-a58327, no binaries in worktree) was visibly honored by the implementer.
