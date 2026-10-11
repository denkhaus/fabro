# Revision — run 01M4KZ2BCS4C17PTYRRPANBHJ6

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M4KZ2BCS4C17PTYRRPANBHJ6.md
- seeds filed: none — healthy run (2 findings duplicates of open seeds; 1 surviving finding zero-credit overflow; 1 finding links to an open overflow)
- balance: 0 non-exempt seeds filed / 0 — no credit this pass (no same-pass stale/superseded closes)
- basis: run 01M4KZ2BCS4C17PTYRRPANBHJ6, workflow version 968b… (recorded truncated by the select stage journal), commit a8de316350b4501cf5e73cb071e47e11160c4465
- revised_at_commit: a8de316350b4501cf5e73cb071e47e11160c4465 (ADR-0015: engine drift signal for later judgement)

## Findings

**1. Planner in-flight exclusion uses the absent `fabro_runs_list` tool** — duplicate_of: fabro-9b84 (open). The run's planner degraded to 26 shell probes (45% of run cost) because no fabro tool materialized in its session despite the node declaring `x.fabro_tools="fabro_run_search"`. fabro-9b84 already names the rename; its proposed graph/prompt consistency lint should also assert the declared tool materializes in the agent session (run-level `agent.fabro_tools=false` in workflow.toml is the likely suppressor). Not filed; evidence noted here for fabro-9b84's implementer.

**2. Unpickable seeds occupy the preflight candidate window** — duplicate_of: fabro-863f (open, assignee fabro). 2 of 5 preflight slots (fabro-fec1, fabro-0248) are human-verification residuals the planner can never claim; this run the claimed seed fabro-9f4a sat OUTSIDE the 5-candidate window and got no already-landed check. fabro-863f's parking-label demand fixes the window automatically. Not filed; supporting evidence for fabro-863f's priority: the claimed seed bypassed the fabro-a32f already-landed guard this run.

**3. `verify.nu` FAIL paths print a bounded HEAD of stderr, not a tail** — overflow (zero credit this pass). Concrete change: in `scripts/verify.nu` lines 110–142, the clippy/compile/nextest FAIL branches print `str substring 0..2000` (a bounded head) of the failing command's stderr; long warning preambles can push the actual error (this run: an E0004 in fabro-automation behind a `FAIL clippy fabro-server` verdict) past the bound. Run the checks through nu's `complete` and print a bounded TAIL of stderr after each FAIL line — the pattern `scripts/qualitygate.nu`'s battery tier already uses. Expected effect: the first red verify carries the real diagnostic instead of a truncated one — saves the blind `cargo clippy -p` re-run plus its LLM round-trip on every red verify. Dedupe verified this pass (seeds searches on `verify` / `stderr`: fabro-7ce7, fabro-a9cc, fabro-6e7f, fabro-574d touch verify.nu but none covers stderr tail capture). Analyst's original broader root-cause claim ("FAIL branches print no captured stderr") was refuted by the tree — stderr IS printed, bounded head-only.

- overflow: `verify.nu` FAIL paths print a bounded HEAD of stderr, not a tail — in `scripts/verify.nu` clippy/compile/nextest FAIL branches, print a bounded TAIL of the failing command's stderr via `complete` (qualitygate battery-tier pattern) instead of `str substring 0..2000`; effect: first red verify carries the real diagnostic (E0004 behind long warning preamble this run), saving a blind cargo re-run plus LLM round-trip per red verify.

**4. Prompt-lint warning noise floor (intentional routing schemas + toolchain date-pins)** — overflow-dup: Silence by-design prompt-lint warnings in the qualitygate (open in 01M2XQJC3TWRMJ1128WRXRQYXH.md). This run's gate output: 20 warnings, 17 routing-named on intentional schemas, 2 date-pin warnings on the load-bearing pinned nightly `2026-04-14`. Same theme already open — linked, not re-journaled.

Open overflows from the ledger were merged as candidates this pass and consolidated against the above; all remain open entries — none could be filed against this pass's zero credit (ADR-0022), and the prompt-lint finding above linked to its already-open sibling rather than duplicating it.
