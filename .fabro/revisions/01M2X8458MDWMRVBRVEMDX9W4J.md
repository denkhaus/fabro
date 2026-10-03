# Revision — run 01M2X8458MDWMRVBRVEMDX9W4J

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2X8458MDWMRVBRVEMDX9W4J.md
- seeds filed: none — healthy run (one finding survived dedupe but had no filing-balance credit)
- balance: 0 non-exempt seeds filed / 0 — no credit this pass (no same-pass stale or superseded closes)
- basis: run 01M2X8458MDWMRVBRVEMDX9W4J, workflow version d45e190e908df8214d42704ce0821f39cfe5bf0cbecfe4662da883c4394640be, commit da934042d0dc1a12d11586150e3891a5daa7fc80
- revised_at_commit: da934042d0dc1a12d11586150e3891a5daa7fc80 (ADR-0015: engine drift signal for later judgement)

## Findings

### Memoize planner external-wait skip adjudications into the seed body

- filed: not filed — overflow to journal (ADR-0022: zero same-pass close credit; `sd search "memoize"` / `sd search "skip"` show no existing seed covering this change)
- concrete change: when the develop planner skips a candidate on an external-wait condition, persist the adjudication as an append-only skip note in the seed body (e.g. `skip note: waiting upstream fabro-sh#784, verified open 2026-09-19, re-check only after upstream merge`); change site is the skip-adjudication step of `.fabro/workflows/develop/prompts/planner.md`, using an append-safe carrier (prefer the note-append command open seed fabro-23d4 demands over a whole-body `sd update` rewrite).
- expected effect: removes the re-derivation chain (~4 tool calls, 2 error recoveries, 13KB `web_fetch`) from every develop lap while fabro-sh#784 stays open; later planners read the adjudication for free from the `sd ready` / `sd show` calls they already make.
- overflow: Memoize planner external-wait skip adjudications into the seed body — append-only skip note in the seed body at the skip-adjudication step of `.fabro/workflows/develop/prompts/planner.md`; effect: removes recurring per-lap re-derivation tax (~4 tool calls, 2 error recoveries, 13KB fetch) on upstream-PR waits. Next pass may re-file against its own balance after re-running dedupe (checked this pass: no duplicate — fabro-af22 tracks the fix itself, fabro-4bb7/fabro-9372 cover the runs-list projection).
