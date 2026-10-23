# Revision — run 01M5KCXWDYM96AVJM95S51A1Y3

- status reviewed: succeeded(completed)
- review: .fabro/reviews/develop/01M5KCXWDYM96AVJM95S51A1Y3.md
- items filed: none — healthy run, zero filing credit this pass (tracker binding outage continues)
- balance: 0 non-exempt items filed / 0 — no credit this pass (no same-pass stale or superseded closes; the only candidate supersession target `fabro-89bc` is already closed, so finding 3 is a regression follow-up, not a supersession)
- basis: run 01M5KCXWDYM96AVJM95S51A1Y3, workflow version absent, commit 8d79fc9c84f35b392403cc72c05e1c22b2ed91e4
- revised_at_commit: 8d79fc9c84f35b392403cc72c05e1c22b2ed91e4 (ADR-0015: engine drift signal for later judgement)

## Findings

**1. Pin the planner's tool-call contracts (run-status enum)** — overflow-dup: Pin the canonical fabro_run_search invocation in planner prompt (open in .fabro/revisions/01M5KEMT84HHVYEZDS1KZC5S7E.md)

This run's planner re-errored the same call shape (fabro_run_search 2 calls / 1 error on `in_progress, queued, pending, running`; fabro_run_set_title 3 calls / 2 errors on the 100-char cap): live second occurrence strengthening the already-open overflow entry. Title-cap half judged duplicate_of: fabro-b529 (open, covers the cap itself). Not re-journaled as a full overflow entry — the open entry at the linked file is the single ledger surface for this theme.

**2. Fix the brief-level regression that re-introduced the double battery run** — overflow

- overflow: Fix the brief-level regression that re-introduced the double battery run — replace "parse-level checks (lint-nu, prompt-lint, validate)" in the Envelope note of `.fabro/workflows/loop/prompts/planner.md` with "single-file probes (`nu -c 'source <script>'`, exact-string grep); whole-repo tiers belong to the tester alone"; effect: restores the fabro-89bc (run 01M550CAX) fix — this run's brief said "verification is lint-nu/prompt-lint/validate parse-level" and the implementer ran prompt-lint (77 files), just lint-nu (65 scripts), and check-run-scope, all three re-run by the tester ~3 minutes later, so ~10–15s tool time plus an implementer turn are saved every run.

Original fix item fabro-89bc is closed; no open item covers the brief-vocabulary regression (seeds grep on "single-file probes"/"battery" this pass).

**3. Briefs must state precondition status (met / waived) for named dependency seeds** — overflow

- overflow: Briefs must state precondition status for named dependency seeds — in `.fabro/workflows/loop/prompts/planner.md` step 7, when the seed body names other seed ids, add one brief bullet per named precondition: `met` / `waived because <one line>`; effect: precondition risk surfaces at brief time and is reviewable instead of being silently re-decided mid-implementation (this run: fabro-1923's action was conditional on fabro-4653, the planner had adjudicated the waiver in its journal, but no brief reached the implementer context — only claim_check's ok line).

Distinct from open fabro-30a5 (tracks the fabro-4653 dependency itself, filed by this run's closeout); seeds grep on "precondition" matched only unrelated fabro_run_search items.

**4. File the architect-lane vocabulary drift both roles flagged** — overflow

- overflow: Migrate the architect lane's analyze prompt to the ADR-0032 item-key vocabulary — touch `.fabro/workflows/architect/prompts/analyze.md` lines 13/15/67 as one unit (line 67 still requires "a known seed id … new-seed justification", the retired seeds-CLI vocabulary) when that lane migrates to `fabro_work_*`; effect: closes the last lane speaking the retired vocabulary to analysts, so Ask-Fabro deduped recommendations stay uniform across lanes (implementer AND reviewer of run 01M5KCXWDYM96AVJM95S51A1Y3 independently journaled the drift as out of fabro-1923's scope).

Priority: 2 (low — lands with the architect lane's tool migration). Seeds grep on "architect" matched only unrelated architect-workflow items; fabro-4653/fabro-30a5 cover the loop lane only.

Dropped as duplicates (analyst visit-2, re-verified open this pass): planner firehose → fabro-c3b4; reviewer double lint read → fabro-17fb + fabro-a827; English implementer responses → fabro-eaa0; title-cap half of finding 2 → fabro-b529. Closeout-race re-check (degraded, seeds grep — tracker binding outage, 5th consecutive context): only fabro-30a5 cites the run id; its body covers none of the candidates.
