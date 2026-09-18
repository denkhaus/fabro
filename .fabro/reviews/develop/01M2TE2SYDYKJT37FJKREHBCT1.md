# Improve review — run 01M2TE2SYDYKJT37FJKREHBCT1

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (9.9 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-18 14:35+0000 by revisor `fabro_ask`

---

All evidence gathered — from run events (planner transcript, timings, usage), worker logs, the journal, and the workspace scripts (`planner-preflight.nu`, `dup-run-check.nu`) plus the seed tracker. Here is the analysis.

## What this run actually was

A **bookkeeping-only close**: preflight (11.7 s, script) → planner LLM lap (559 s wall, $0.263 — 100% of run cost, 15 tool calls) → "Already landed" exit. The planner existed only to re-derive one greppable fact: fix commit `47eedbec1` was already in `origin/denkhaus` — it was literally **this run's own base SHA**. Total: 576 s wall / $0.263 / PR #238 (diff = journal + tracker only). The tester, implementer, and reviewer never ran.

## Recommendations, by expected impact

**1. Make landed-detection count direct (non-PR) fix commits — `.fabro/scripts/dup-run-check.nu`, lines 225–227.**
What happened: `landed` is built from 2-parent merges ∪ squash subjects ending `(#n)` only; the human's direct commit `47eedbec1` ("…(fabro-c42f)", single parent, no `(#n)`) fell into `$other`, so the top candidate came back `clean` and the preflight's mechanical superseded-close never fired. The planner then spent 15 tool calls / 559 s / $0.263 re-deriving it (run events seq 74–88), ending in exactly the close the script was designed to do itself (fabro-a32f). Change: classify single-parent base-branch commits whose subject/body references the seed id as implementation matches (a body-match qualifier already exists at line 170). Expected effect: this entire run class (~9.6 min, $0.26 each) collapses to a ~30 s, zero-LLM preflight exit.
*New-seed justification: fabro-ead4 covers the opposite error (filed-only false positives) plus non-top closure; closed fabro-a32f introduced the node but its landed class is PR-merge-only; fabro-a01f is the claim race — no open seed demands counting direct base-branch fix commits.*

**2. Warm the run container's Rust build — seed fabro-c643** (open, priority 1, user GO 2026-09-17, `.fabro/Dockerfile.toolchain`).
What happened: the planner's closure-evidence call `cargo nextest run -p fabro-workflow routing_contract` took **290,172 ms wall for a test that itself ran in 0.011 s** (run events seq 112–114) — 50.4% of the run's total wall time was cold compilation of 3 binaries. Change: the cargo-chef/sccache warm-cache arms already consolidated in fabro-c643. Expected effect: 4–5 min off every run that makes any compile-level probe; this lap would have been ~10 s instead of 4 m 50 s.

**3. Cost-tier the Already-landed closure evidence — `.fabro/workflows/develop/prompts/planner.md`, ALREADY-LANDED arm.**
What happened: planner.md itself says "No cycle runs: the fix is proven landed, a verification lap re-proves nothing," yet the planner still ran the 290 s cold regression pin as "closure evidence" — the prompt neither sanctions nor forbids a compile-level probe on this route. Change: one sentence — closure evidence = commit diff inspection (git show) + the landed commit's own test suite; compile-level probes are forbidden on the Already-landed route. Expected effect: removes the 290 s even before any cache warming; complements fabro-c643 rather than depending on it.
*New-seed justification: fabro-4be6 is brief verification dry-runs, fabro-7f58 is implementer test scoping, fabro-c643 is the cache — none sets a cost tier for planner-side closure evidence.*

**4. Extend the mechanical superseded-close to non-top duplicates + fix the filed-only classifier — seed fabro-ead4** (`.fabro/workflows/develop/scripts/planner-preflight.nu`).
What happened: this run's verdict table **again** listed fabro-90ae as `duplicate` (sha 4f8a254, #232) as candidate #3 with `closed.seed=null` — the closure arm only fires for the TOP candidate (script line 245), so the seed stays open and every run re-reports it; and per fabro-ead4's fresh evidence, #232 only *filed* the seed (the subject shape "file fabro-90ae (split tests.rs)" matches none of `classify-filed`'s regex branches in `dup-run-check.nu` line 75, so it was wrongly counted as an implementation). Expected effect: fabro-90ae stops being re-adjudicated in every planner preamble, and the false-duplicate class that risks wrongful closes is closed.

**5. Top-N `sd ready` view instead of the firehose — seed fabro-c3b4** (planner prompt / `sd` call shape).
What happened: the planner's first call returned **200 seeds / ~29 KB stdout, truncated in the tool result** (run events seq 37–39, `stdout_truncated: true`); it used only the top candidate, which the preflight had already ranked. Expected effect: smaller first-round context, cheaper and faster planning laps.

**6. Skip the project gate on loop-asset-only run PRs — seed fabro-9495** (publish/postlude).
What happened: PR #238 was created with `auto_merge: enabled` for a diff that is entirely `.fabro/journal/…` + `.seeds/issues.jsonl` (4 additions, 1 deletion) — a full gate/CI cycle to deliver a tracker close. Expected effect: bookkeeping PRs like #238 merge in minutes without gate spend; real code PRs keep the gate.

**7. Annotate bookkeeping-only runs in `run.completed` — seed fabro-b1d3** (notifications).
What happened: `run.completed` fired to `#dev-fabro` (14:27:49) for a tracker-only close indistinguishable from a development run. Expected effect: the Slack consumer can triage $0.26 bookkeeping passes vs. real implementation passes at a glance.

**8. Quote-safe probe commands — seed fabro-b6f9** (planner probe discipline).
What happened: seq 98–100 — `sed -n "$(rg -n … | cut -d: -f1),+40p" …` failed ("unknown command: `\n`") because the substitution returned multiple lines; it was the lap's only tool error (13 shell calls, 1 error) and spawned the `unsupported_control` provider warnings. Expected effect: −1 wasted round per occurrence; fewer cascading provider warnings.

**9. PR-body generation non-strict — seed fabro-41b1** (PR postlude).
What happened: worker log 14:27:38 — "PR content structured generation failed; retrying once without strict JSON output … the model did not return a JSON document"; the retry masks the failure (it recovered here, but silently). Expected effect: deterministic PR bodies, no masked-retry path.

**10. Absent optional memory files at info, not ERROR — seed fabro-a701** (agent session init).
What happened: 2× ERROR at session init for `/workspace/fabro/.codex/instructions.md` not found — a missing optional file, logged as error on every planner session. Expected effect: warn/error channels carry only actionable signals.

**Sources**: run events (stage transcripts seq 26–140, timing/usage blocks, journal painpoint), worker warn/error log, and workspace files `.fabro/workflows/develop/scripts/planner-preflight.nu`, `.fabro/scripts/dup-run-check.nu`, `.seeds/issues.jsonl` (seed coverage checks). Not inspected: the tester/gate transcript (never ran in this run) and PR #238's CI outcome (post-run, outside event scope).

## Revisor distillation (post-tracker-check)

- dup-run-check fabro-c42f --self 01M2TE2SYDYKJT37FJKREHBCT1: verdict clean (self-closure, PR #238 Fabro-Run trailer)
- dropped as duplicate_of open seeds: rec2->fabro-c643, rec4->fabro-ead4, rec5->fabro-c3b4, rec6->fabro-9495, rec7->fabro-b1d3, rec8->fabro-b6f9, rec9->fabro-41b1, rec10->fabro-a701
- survived: rec1 (new - direct-commit landed detection), rec3 (new - route-specific closure-evidence cost tier)
