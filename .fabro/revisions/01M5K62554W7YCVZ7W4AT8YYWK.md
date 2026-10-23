# Revision — run 01M5K62554W7YCVZ7W4AT8YYWK

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M5K62554W7YCVZ7W4AT8YYWK.md
- items filed: none — zero balance credit this pass; all surviving findings journaled as overflow below
- balance: 0 non-exempt items filed / 0 — no credit this pass (no same-pass stale or superseded closes; fabro-9e2c and fabro-17fb are complementary, cross-referenced, not superseded)
- basis: run 01M5K62554W7YCVZ7W4AT8YYWK, workflow version absent, commit 7d75eae52676e9ce34835a650b6e029e7432edf7
- revised_at_commit: 7d75eae52676e9ce34835a650b6e029e7432edf7 (ADR-0015: engine drift signal for later judgement)

## Findings

Dropped as duplicates at analyze time (verified open in `.seeds/issues.jsonl`): `rg -r` replace-flag footgun → fabro-6997; run-title 100-char cap (3 calls / 2 errors this run) → fabro-b529. Closeout-race re-check at filing time (degraded, seeds grep — tracker query dead): zero seeds cite run 01M5K62554W7YCVZ7W4AT8YYWK.

**1. Closeout: approved verification-only seeds park forever, exit edge lies, and the PARK message prints a literal placeholder (multi-arm)** — overflow

Same-file consolidation: both arms target `.fabro/workflows/develop/scripts/closeout.nu`.

- overflow: Closeout has no closure arm for approved verification-only seeds and the PARK message prints a literal placeholder; effect: approved verification seeds actually close (~$0.24 + 5 min + a PR per saved lap; fabro-30ae still in_progress after PR #470 merged green via the "Seed closed" edge) — arms schema: (arm 1) add an approved-verification arm in `.fabro/workflows/develop/scripts/closeout.nu` closing with a reason citing the run + verdict row when `review_verdict=approved` and the only seed-work file is the tracker record — expected effect: approved verification seeds actually close instead of re-verified every lap; (arm 2) in `.fabro/workflows/loop/workflow.fabro` give the `closeout` node `output_schema="routing"` and split the exit edges into `Seed closed` / `Parked` / `failed` — expected effect: a park can no longer ride the "Seed closed" edge, terminal state stops lying; (arm 3) unescape `$"…\($seed_id\)…"` at `.fabro/workflows/develop/scripts/closeout.nu:536` (escaped parens suppress Nushell interpolation, so the message ALWAYS prints the placeholder; stdout read "PARK — **($seed_id)** NOT closed" at step.finished seq 432 while context held `current_seed_id: fabro-30ae`) — expected effect: park notes name the seed for the next run and for humans; one-line fix. Cross-ref (complementary, NOT supersession): open fabro-9e2c surfaces park state in the terminal event; closed fabro-9d26 built the fast path without a closing arm; closed fabro-02c4 is the pre-close diff-visibility gate.

**2. Evidence capture: reviewer adjudicates a tracker-only diff blind** — overflow

- overflow: Inline the changed tracker record's fields when the per-seed patch is tracker-only; effect: reviewer judges the actual verdict row — in `.fabro/workflows/develop/scripts/evidence.nu`, when the per-seed patch is tracker-only (`.seeds/issues.jsonl +1/-1`), echo the changed record's status + verdict line instead of the empty-hunks placeholder ("(zero per-seed hunks … by design)"; reviewer journal: "the tracker delta itself is unreadable from the capture … the reviewer must adjudicate a +1/-1 stat with no visible content") — expected effect: faster, defensible approvals on every verification-only run. Distinct from open overflow "Render very long diff lines readably" (`01M5CN9EC11K39YCG4HCHDJ6NE.md`) — line length vs missing content.

**3. Loop reviewer prompt: re-executes every recorded check the evidence already inlined** — overflow

- overflow: Verification-only reviewers should judge from the recorded check transcript, re-running at most the single decisive check; effect: roughly halves reviewer stage time/cost on verification passes — in `.fabro/workflows/loop/prompts/reviewer.md`, for Verification-only runs instruct judging from the recorded transcript (this run: 4 grep calls reproducing the planner's greps verbatim, 54.3s / $0.042 of a $0.238 run) — expected effect: ~50% reviewer cost/time saved per verification pass, consistent with closed fabro-50c9's intent. Cross-ref (complementary, NOT supersession): open fabro-17fb and open overflow "Deduplicate what the reviewer reads" (`01M5CR2B9DKQ0ZRXKD3DZSPMP0.md`) dedupe the reviewer's INPUT; this changes the reviewer's re-RUN behavior.

Basis for all bullets: run 01M5K62554W7YCVZ7W4AT8YYWK, workflow version absent, commit 7d75eae52676e9ce34835a650b6e029e7432edf7.
