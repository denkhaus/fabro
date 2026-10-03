# Improve review — run 01M2Q7VVHNQPTTEY14AFPBHFBZ

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (9.2 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-17 08:44+0000 by revisor `fabro_ask`

---

All facts below are from this run's stage records, journal, and terminal diff (via run inspection), plus seed checks against `.seeds/issues.jsonl` in the workspace. Run shape: verification-only pass closing `fabro-d97b`; 528 s wall, $0.453 total; planner = 409 s wall / $0.402 (78% of run), reviewer = 105 s / $0.051; planner made 30 tool calls (29 shell, 1 `fabro_runs_list`) with 1 shell error. Terminal diff: 2 files, +7/−1 (journal + tracker claim/close flip only), yet PR #211 was created with auto-merge.

## Recommendations, by expected impact

**1. Give the planner the implementer's compile-probe cost-tier rule — the single largest measured waste in this run.**
- What happened: the planner's probe `cargo run -q -p fabro-cli -- parse .fabro/workflows/develop/workflow.fabro` timed out at 180 s (cold compile — `target/debug/incremental/` debris is still in the workspace; from planner journal + the stage's 188 s tool time / 1 shell error). That one dead call was ~34% of run wall; the planner then fell back to grep/awk anyway. The implementer prompt already has this rule family (hard rule (a) + "never `cargo run` cold; `cargo build` once with timeout_ms >= 600000"), but `planner.md` contains zero timeout guidance (grep-confirmed: only a cheapest-first ordering line at ~line 40).
- Change: add to `.fabro/workflows/develop/prompts/planner.md` (STALE-BASIS CHECK / brief-writing area): probe cost-tier — grep/parse-level verification before any build-level probe; compile/test probes must pass `timeout_ms >= 600000`; never `cargo run` cold.
- Expected effect: removes the observed 180 s dead probe (and its guaranteed shell error) from every planner lap that verifies graph-level criteria.
- Seed: **new seed** — no existing seed covers planner probe timeouts/cold-compiles (fabro-55a7 = batching, fabro-5bbd = output labeling, fabro-4be6 = dry-running brief commands; none mentions timeout_ms or cold builds).

**2. Extend fabro-9f97's skip condition to tracker churn — this run is the proof it never fires for verification-only runs.**
- What happened: PR #211 was created (auto-merge enabled) for a diff of only `.fabro/journal/<run>.jsonl` (+6) and `.seeds/issues.jsonl` (+1/−1). fabro-9f97 skips the PR only when the diff *excluding `.fabro/journal/`* is empty — the tracker flip makes it non-empty, so the seed as written cannot cover verification-only/bookkeeping runs.
- Change: `sd update fabro-9f97 --description` amending the condition to exclude `.fabro/journal/**` **and** `.seeds/**` (matching fabro-b1d3's "bookkeeping-only" definition), implementation still in the pull_request settings; cite run 01M2Q7VVHNQPTTEY14AFPBHFBZ / PR #211 as added evidence.
- Expected effect: verification-only and no-op runs stop producing PRs, auto-merges, and squash-merged journal/tracker churn into `denkhaus`.
- Seed: **fabro-9f97** (open, priority 1) — amended, not duplicated.

**3. Batch planner reconnaissance into one shell call — this run re-confirms the pattern fabro-55a7 targets.**
- What happened: 29 shell calls / 30 messages / 220.6 s planner inference for one verification-only claim (in-flight check alone is multi-round: `fabro_runs_list` plus journal-fallback greps).
- Change: implement fabro-55a7 in `.fabro/workflows/develop/prompts/planner.md` (combined `sd ready` + `sd show` + greps in one call).
- Expected effect: per the seed's own measurement, ~40–60 s and ~$0.06 per planner pass; also shortens the claim-to-PR race window.
- Seed: **fabro-55a7** (open).

**4. Fix the stale fabro-9467 comment in workflow.fabro — both LLM stages flagged it and no role is allowed to fix it.**
- What happened: the reviewer-node comment ("fabro-9467: the reviewer is a prompt node (no tools)") contradicts the now-agent reviewer; the planner journaled it and had to put a "known annotation, not a change mandate" waiver in the brief, and the reviewer journaled it again — two journal entries plus brief overhead adjudicating one stale sentence. Verification-only passes make no changes and the implementer's report-don't-fix rule bars incidental loop-asset edits, so it will keep recurring.
- Change: one-line comment correction in the `reviewer` node block of `.fabro/workflows/develop/workflow.fabro` (the `preamble_inline_max_kb` comment).
- Expected effect: future planner briefs and reviewers stop re-adjudicating the node-type contradiction.
- Seed: **new seed** — fabro-9467 (closed) covered the inline-budget feature, not comment hygiene; no open seed names this cleanup.

**5. Annotate bookkeeping-only runs in Slack — this run fired an ordinary run.completed for a journal+tracker-only PR.**
- What happened: notifications are configured (`run.completed` → `#dev-fabro`); this run's only output was PR #211 with a 2-file bookkeeping diff, indistinguishable in notification from a code run.
- Change: implement fabro-b1d3 (annotate payload when the diff touches only `.fabro/journal/**` and `.seeds/**`).
- Expected effect: humans triage no-op runs in Slack without opening the PR.
- Seed: **fabro-b1d3** (open).

Not inspectable from here: PR #211's actual check/merge outcome (`pull_request.state` was null in the run summary) and the per-call planner transcript beyond the aggregate counts — so gate-cost claims for #2 rely on fabro-9495's recorded ~15 min measurement, not this run's events.
