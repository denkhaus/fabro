# Revision — run 01M59JSC3MKGRCH5PMR53XHADZ

- status reviewed: succeeded (revisor pass, 907 s wall, $0.068 total)
- review: .fabro/reviews/develop/01M59JSC3MKGRCH5PMR53XHADZ.md
- seeds filed: none — zero balance credit this pass; 4 surviving findings journaled as overflow below
- balance: 0 / 0 — no credit this pass (no same-pass stale/superseded closes; fabro-6b0f closeout coverage is not credit)
- basis: run 01M59JSC3MKGRCH5PMR53XHADZ, workflow version fb1f7f607b87f1d7a46429700779b96154992a343bd3240cd95aa39120df9928, commit 9ee8ef9a6ef16f0ca274c40dfaf735bd6bc9ee39
- revised_at_commit: 9ee8ef9a6ef16f0ca274c40dfaf735bd6bc9ee39 (ADR-0015: engine drift signal for later judgement)

## Findings

**1. Cut implementer turn count via a check-scaffold** — overflow

- overflow: Cut implementer turn count via a check-scaffold — in `.fabro/workflows/loop/prompts/implementer.md`, prescribe one scaffolded checks file (single write, per-criterion checks as functions) run through `check-transcript.nu` once instead of six ad-hoc per-criterion scripts across six-plus shell calls; effect: every avoided turn saves ~22 s (622 s inference / 28 turns ≈ 22 s per turn), realistic 3–5 minutes off every loop implement run.

**2. Fix seed-spec-paths extraction in the loop gate** — overflow

- overflow: Fix seed-spec-paths extraction in the loop gate — in `.fabro/workflows/loop/scripts/loop-gate.nu` (shared extraction), make the seed-spec-paths tier ignore tracker-path citations inside anecdote/correction lines and count directory-level citations (the body names `.fabro/workflows/develop/prompts/`); effect: no false REDs (a bounce costs a full ~11-min implementer cycle) and the tier actually verifies the five prompt work files this run only passed by luck.

**3. Stop re-citing dead paths in correction notes** — overflow

- overflow: Stop re-citing dead paths in correction notes — in `.fabro/scripts/claim-check.nu`, ignore paths inside "Correction …" lines (or phrase corrections in `.fabro/workflows/loop/prompts/planner.md` without re-citing the dead path); effect: clean claim-check output after `--set-description` corrections and protection against a future promotion of that flag to blocking, which would deadlock the correction loop.

**4. Mirror the per-role table split into the loop lane's prompts** — overflow

- overflow: Mirror the per-role table split into the loop lane's prompts — apply the `tracker-commands-{planner,implementer}.md` include pattern to `.fabro/workflows/loop/prompts/project-facts.md`, which still ships the full 6-row command table to all three loop roles; effect: same prompt savings fabro-52b4 measured in develop (−1,692 B reviewer / −1,335 B implementer) and no write-command advertising to the loop reviewer/implementer.

**5. Trailing-newline check in prompt-lint** — closeout-covered duplicate

- duplicate_of: fabro-6b0f — the target run's own closeout filed this at 08:15 (verified via closeout-race re-check); no filing claim remains.

**6–8. Adopt-known-seed recommendations (no filing)**

- fabro-9588 (per-node AGENTS.md memory scoping): this run MemoryLoaded 28,476 bytes of `/workspace/AGENTS.md` despite fabro-9588 being closed 2026-10-11 — possible non-deployment/regression on the loop lane; flagged to journal.
- fabro-c3b4 (top-N `seeds ready` view): planner's first tracker call returned the full 58-row firehose (8,440 bytes retained) after preflight had shortlisted 5.
- fabro-b769 (long-stage heartbeat): implementer ran 10 m 40 s with zero surfaced progress; notifications only fire `run.completed`/`run.failed`.
