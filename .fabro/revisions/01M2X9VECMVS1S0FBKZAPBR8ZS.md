# Revision — run 01M2X9VECMVS1S0FBKZAPBR8ZS

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2X9VECMVS1S0FBKZAPBR8ZS.md
- seeds filed: none — pass had 0 filing credit (0 same-pass stale/superseded closes); all 3 surviving findings journaled as overflow for the next pass
- balance: 0 non-exempt seeds filed / 0 — no credit this pass
- basis: run 01M2X9VECMVS1S0FBKZAPBR8ZS, workflow version f14d776bb31ba263a27fcabf5ccf9d2d4c8a4e38dc7cc89fc2d9d391b9675030, commit 72ba843a6ef398c08a0f28992cb9eae16ba7e10e
- revised_at_commit: 72ba843a6ef398c08a0f28992cb9eae16ba7e10e (ADR-0015: engine drift signal for later judgement)

## Findings

### Verify section-heading and mx-id citations in planner-preflight.nu pre-claim
- filed id: none — overflow-to-journal (0 credit this pass)
- Concrete change: in `.fabro/workflows/develop/scripts/planner-preflight.nu`, add a check arm that greps quoted section-heading citations from seed bodies against the named file and validates cited mx-ids via `ml search`, surfacing results as anchor_flags. Distinct from closed `fabro-7daf` (file:line anchors) and open `fabro-4c81` scope. Expected effect: stale-anchor seeds flagged sub-second pre-claim instead of burning a claim/close cycle.

### Verify ml record --name upsert semantics behind the one-record-per-lesson rule
- filed id: none — overflow-to-journal (0 credit this pass)
- Concrete change: run one quiet-run probe of mulch CLI 0.10.7 (file + re-file with same `--name`, diff the expertise jsonl) to confirm `ml record --name` upserts by merging outcomes; if not, soften the amend rule in `.fabro/workflows/develop/prompts/implementer.md` (from `fabro-fe6f`, PR #287). Expected effect: one-record-per-lesson rule becomes enforceable or is corrected before the next cycle regresses.

### Add a waiver channel for the load-bearing date-pin lint warning
- filed id: none — overflow-to-journal (0 credit this pass)
- Concrete change: teach the project-facts date-pin lint to honor an explicit acknowledgment marker (e.g. `lint-waiver: date-pin ... reviewed <date>` line in `project-facts.md`). Expected effect: gate warnings drop to actionable ones only; real pin drift regains signal.
