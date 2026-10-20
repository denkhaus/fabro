# Revision — run 01M5CN9EC11K39YCG4HCHDJ6NE

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M5CN9EC11K39YCG4HCHDJ6NE.md
- seeds filed: none — healthy pass (fabro-16a7 open, dup-run-check clean; 0 balance credit this pass, surviving findings journaled as overflow)
- balance: 0 non-exempt seeds filed / 0 — no credit this pass
- basis: run 01M5CN9EC11K39YCG4HCHDJ6NE, workflow version absent, commit 9466581c7ab21a0191b48cf2ae89bf957cd29c19
- revised_at_commit: 9466581c7ab21a0191b48cf2ae89bf957cd29c19 (ADR-0015: engine drift signal for later judgement)

## Findings

Dropped as duplicates (analyzer-verified; closeout-race re-check ok count 0):
- whole-repo battery rule port to loop implementer+reviewer prompts — duplicate_of: fabro-915a (also open overflow "Scope implementer pre-gate checks to the diff, not the whole repo" in 01M550CAXV85JGT1XBPCDRE5DY.md; this run adds evidence: implementer ran `just lint-nu` 67 scripts + `prompt-lint` 81 files, both re-run by tester ~90s later)
- planner top-N candidate view — duplicate_of: fabro-c3b4 (58-row `seeds ready` firehose, 8.4 KB into planner context)
- title-cap retry friction — duplicate_of: fabro-b529 (`fabro_run_set_title` 3 calls / 2 errors on 105-char titles; error-ergonomics half adjacent to fabro-e4ac)
- long-stage heartbeat — duplicate_of: fabro-b769 (implementer 5m21s, 65% of wall, silent)

**1. Fix the check-transcript bare-literal parse trap** — overflow
- overflow: Fix the check-transcript bare-literal parse trap — in `.fabro/workflows/develop/scripts/check-transcript.nu` (line 67), either type the rest param looser and stringify so bare `true`/`false` literals survive `nu::parser::parse_mismatch`, or add a header comment plus a line in both lanes' implementer prompts (next to NESTED-SHELL PROBES) that check commands must not be bare nu literals (`sh -c 'exit 0'` style); effect: no more parse-class exit-1 records from well-formed checks (1 of only 2 errored tool calls this run) and fewer first-attempt/re-run transcript pairs. Closed fabro-e32c covers quoting mangling only; cross-ref open overflow "Flag consecutive same-command records with differing exit codes" (01M5CPBDF5PNC8EGDK805SVW10.md) — same file, different failure class.

**2. Render very long diff lines readably in the evidence capture** — overflow
- overflow: Render very long diff lines readably in the evidence capture — in the diff rendering of `.fabro/workflows/develop/scripts/evidence.nu`, cap or fold individual lines past ~200 chars with an explicit `long line elided — use read_file` marker instead of truncating mid-hunk; effect: reviewer verifies prompt-file changes from context (this run's 1,700+ char single-line hunk forced the reviewer stage's only 2 tool calls as rescue reads). fabro-533c (binary-classified diffs) and fabro-9a43/fabro-eed2 (reviewer-side coping) cover different mechanisms; verified not a duplicate.

Linked to already-open overflows (no new entry per ADR-0022 rule 3):
- overflow-dup: Silence by-design prompt-lint warnings in the qualitygate (open in 01M2XQJC3TWRMJ1128WRXRQYXH.md) — this run's prompt-lint flood measurements (35 warnings + ~26 notes: 17 date-pin files, 20 routing-named warnings across 4 schemas, synthetic-id lists; 8.7 KB tester output, repeated in the 38 KB evidence capture) attach to it; the alternative fix home (aggregate repeats inside `.fabro/scripts/prompt-lint.nu`, cross-consolidable with the open "Batch prompt-lint synthetic-id fixture notes" entry in 01M5CPBDF5PNC8EGDK805SVW10.md) is recorded here for the next crediting pass.
