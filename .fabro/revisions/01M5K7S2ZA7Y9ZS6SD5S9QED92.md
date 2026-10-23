# Revision — run 01M5K7S2ZA7Y9ZS6SD5S9QED92

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M5K7S2ZA7Y9ZS6SD5S9QED92.md
- items filed: none — healthy run (bookkeeping-only run; closeout gap itself is already an open overflow)
- balance: 0 non-exempt items filed / 0 — no credit this pass
- basis: run 01M5K7S2ZA7Y9ZS6SD5S9QED92, workflow version absent, commit adeaefdb11de0ab9a69d08ff35a3bcc5eb39890a
- revised_at_commit: adeaefdb11de0ab9a69d08ff35a3bcc5eb39890a (ADR-0015: engine drift signal for later judgement)

## Findings

**1. Make already-landed verifications machine-detectable in preflight** — overflow

- Change: `.fabro/workflows/develop/scripts/planner-preflight.nu` — extend the duplicate/already-landed arm to also scan `.fabro/journal/<run_id>.jsonl` for the candidate seed id when no commit message references it, and emit it as an advisory verdict.
- Effect: this run reported fabro-30ae `"verdict":"clean"` while commit `c3eea6563` sat in base (it never referenced the seed id); the planner re-derived the verification itself (255s, $0.24, 26 tool calls). A journal pointer would have routed "Already landed" at tracker_guard for zero LLM tokens instead of a 253s inference stage.
- Dedupe: analyst searched `preflight landed duplicate` (only closed fabro-7280, wrong base branch) and `journal duplicate already-landed` (0); no open overflow in the ledger matches journal-derived duplicate detection — "Salvage stranded implementations" (`01M2YHB8B3791JWWKYK88B08V7.md`) greps unmerged branches, a different source. NOT a duplicate of overflow #1 "Closeout has no closure arm for approved verification-only seeds" (`01M5K62554W7YCVZ7W4AT8YYWK.md`): that closes the gap at closeout, this detects it at preflight — complementary; this run is realized-cost evidence for both.

Dropped as duplicates (analyst, verified): review recs 3–7 → open fabro-c3b4, fabro-b529, fabro-f18a, fabro-b769, fabro-eaa0; rec 1 → open overflow #1 in `01M5K62554W7YCVZ7W4AT8YYWK.md` (this run is its realized-cost evidence: 255s planner, $0.24, 26 tool calls, one whole follow-up run). Painpoint drops: tracker-bind → fabro-4653; grant-outcome journaling → fabro-3ec9; backlog re-measure nudge → fabro-27bb.

- overflow: Make already-landed verifications machine-detectable in preflight — scan `.fabro/journal/<run_id>.jsonl` for the candidate seed id in the duplicate/already-landed arm of `.fabro/workflows/develop/scripts/planner-preflight.nu` when no commit message references it, emitting an advisory verdict; effect: "Already landed" routes at tracker_guard for zero LLM tokens instead of a 255s/$0.24 re-derivation stage (this run, fabro-30ae vs commit `c3eea6563`).
