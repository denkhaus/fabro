# Revision — run 01M2NMBCRGQAMT2YVRKEH881K6

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2NMBCRGQAMT2YVRKEH881K6.md
- seeds filed: fabro-4be6 — Planner: dry-run every literal verification command in a brief before it ships
- basis: run 01M2NMBCRGQAMT2YVRKEH881K6, workflow version ed2ea157f5427586f9407c99360442db1db24ab8f232704a852f89d8823165db, commit a10595150bc1a3942b8625dcbb6caedea9bea2cf
- revised_at_commit: a10595150bc1a3942b8625dcbb6caedea9bea2cf (ADR-0015: engine drift signal for later judgement)

## Findings

- Planner: dry-run every literal verification command in a brief before it ships — filed fabro-4be6.
  The run's brief transcribed `nu --ide-check scripts/qualitygate.nu`, which errors ('Provide a whole number for this option'); the `;`-chained probe still exited 0, so the failure was stderr-only and cost the implementer one masked failure plus one recovery round (events seq 97). Change: extend `.fabro/workflows/develop/prompts/planner.md` step 7 to dry-run or annotate every literal verification command before the brief is forwarded (step 7 today only confirms paths and headings exist). Effect: eliminates one failed shell call and one LLM recovery round per brief carrying a transcribed-broken command. Not a duplicate: fabro-3805 (scan-scope runnability), fabro-4c81 (path resolution), fabro-cf76 (existence, closed), fabro-e137 (command choice) are different mechanisms.
