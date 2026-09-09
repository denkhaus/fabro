# Revision — run 01M23Q0VM3YVECMSHDNX2HH3PJ

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M23Q0VM3YVECMSHDNX2HH3PJ.md
- seeds filed: fabro-7f58 (run added tests by name filter; full crate suite only when editing existing tests), fabro-dc81 (postlude: re-resolve [run.pull_request] from the run branch tip or emit a takes-effect-next-run notice), fabro-b6f9 (planner probe discipline: never 2>/dev/null probes, always quote sed ranges)
- basis: run 01M23Q0VM3YVECMSHDNX2HH3PJ, workflow version 3b17faf145735aa14caf619a38c0b079a1ff1c91acf24211c9ab08a2612c6f6f, commit 30653c1043a48ac2c1834c761492b5ac10ea11d9
- revised_at_commit: 30653c1043a48ac2c1834c761492b5ac10ea11d9 (ADR-0015: engine drift signal for later judgement)

## Findings

### Run added tests by name filter; full crate suite only when editing existing tests

- filed: fabro-7f58
- Change: scope the test-file-touched crate rule in `.fabro/workflows/develop/prompts/implementer.md` step 4 — full `cargo nextest run -p <crate>` only when the seed edits existing tests; for purely added tests, run the new tests by name filter. Mirror in `.fabro/workflows/develop/prompts/planner.md` step 6 so briefs stop prescribing the full suite.
- Expected effect: ~1 min wall plus one LLM turn saved on every test-adding seed with gate coverage unchanged. Basis: implementer ran all 1543 tests (~56s, seq 253) to validate 2 added unit tests the deterministic tester re-ran green 23s later; goes beyond closed fabro-c4ce (workspace→crate scoping), narrowing further by test intent.

### Postlude: re-resolve [run.pull_request] from the run branch tip or emit a takes-effect-next-run notice

- filed: fabro-dc81
- Change: in `lib/components/fabro-workflow/src/pipeline/publish.rs`, either re-resolve `[run.pull_request]` from the run branch tip when the run's own diff edits its `workflow.toml`, or append a 'takes effect next run' notice to the PR body.
- Expected effect: no misleading failure/warn on the run that ships a fix. Basis: the run-spec snapshot at creation (event seq 1) carried `auto_merge = true` — the exact value seed fabro-6a5a set to `false` — so PR #107's postlude executed under pre-fix settings. Distinct mechanism from fabro-6a5a (config value vs engine resolution) and from fabro-9f97/fabro-1409.

### Planner probe discipline: never 2>/dev/null probes, always quote sed ranges

- filed: fabro-b6f9
- Change: one line in `.fabro/workflows/develop/prompts/planner.md` step 3: never append `2>/dev/null` to probe commands; always quote sed ranges (`sed -n '40,70p'`).
- Expected effect: eliminates silent-empty-probe failures at zero cost. Basis: the planner's basis probe (seq 45/51) began with a malformed `sed -n 40..70p 2>/dev/null;` that returned silently empty and was saved only by the correctly-quoted second sed in the same command. Orthogonal to fabro-8d4c (probe method) — this is shell-quoting/suppression discipline.
