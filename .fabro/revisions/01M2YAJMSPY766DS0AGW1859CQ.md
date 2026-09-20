# Revision — run 01M2YAJMSPY766DS0AGW1859CQ

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2YAJMSPY766DS0AGW1859CQ.md
- seeds filed: none — zero balance credit this pass (no same-pass stale/superseded closes)
- balance: 0 non-exempt seeds filed / 0 — no credit this pass
- basis: run 01M2YAJMSPY766DS0AGW1859CQ, workflow version bdd056c377ba821c9d6cf0cd764c7379961019561cf7e1151acf0b7bec5da4ad, commit 702940e0ec6f321c216347d3e6a1399e63812b6a
- revised_at_commit: 702940e0ec6f321c216347d3e6a1399e63812b6a (ADR-0015: engine drift signal for later judgement)

## Findings

### Closeout: verify each sd close landed before the node exits
- overflow: Closeout: verify each sd close landed before the node exits — in `.fabro/workflows/develop/scripts/closeout.nu`, after every `sd close <id>` re-read `sd show <id>` and hard-fail the closeout node (soft exit, seed stays visibly open) or retry once when the seed is still open; effect: silent close misses become loud failures at close time, eliminating duplicate re-adjudication runs like this one (run 01M2Y8QYY's close of `fabro-ae57` silently never landed; PR #306 spent $0.209 / ~2 min repairing it). Distinct from closed fabro-02c4 (pre-close diff-visibility gate) and open fabro-a81d (batching the close write pair): this is post-close verification. Dedupe checked this pass (sd searches on closeout / requeue): no existing seed names post-close verification.
- filed id: none — overflow (zero credit); NEXT PASS: re-run dedupe, then file

### Tracker guard: requeue immediately when the claiming run is terminal-failed with no PR
- overflow: Tracker guard: requeue immediately when the claiming run is terminal-failed with no PR — in `.fabro/workflows/develop/scripts/tracker-guard.nu`, treat a claim as requeueable IMMEDIATELY (no 6h stale clock) when the claiming run is terminal-failed with no PR, proving terminality from run state/PR absence rather than the journal-closeout shape of closed fabro-32db; effect: priority inversion capped at one run instead of ~5 hours (run 01M2YAJM events seq 49: failed run 01M2Y71ZR33 left High `fabro-af22`/`fabro-b03f` pinned in_flight until the ~07:32 window). Complements open fabro-ab93 (preflight in_flight view-side terminality): this flips tracker state so `sd ready` re-offers the seed.
- filed id: none — overflow (zero credit); NEXT PASS: re-run dedupe, then file
