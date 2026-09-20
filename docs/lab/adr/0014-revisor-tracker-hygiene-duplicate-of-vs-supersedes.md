# Revisor tracker hygiene: duplicate_of vs supersedes

Decided in the 2026-09-03 grill-with-docs session (revisor run analysis,
fabro-2361). Extends ADR-0011's revisor loop with enforceable tracker
semantics, because the first production pass (run 01M1HKRE686W833SE86D8WD319,
10 cycles) filed findings whose near-duplicates the bookkeeper could only
merge away, while superseded seeds stayed open with a SUPERSEDES note in
their description — the tracker degrades silently and `sd ready` keeps
offering stale work (fabro-83f9/fabro-35ab vs fabro-8d2c).

## Decision

Two distinct relations, one actor each:

- **`duplicate_of`** — the ANALYZER drops a finding that names the same
  concrete change as an existing seed (open or closed; closed means the
  change is already implemented). The dropped finding never becomes a seed;
  the journal records `duplicate_of: <id>`.
- **`supersedes`** — the FILER (bookkeeper) closes an open seed with
  `sd close <old-id> --reason "superseded by <new-id>: ..."` when, and only
  when, the new seed replaces the SAME target (same file/mechanism) with a
  strictly better solution. Thematic overlap is not supersession: it gets a
  cross-reference in the description and closes nothing.

The bookkeeper's command reference therefore includes `sd close` and
`sd update` (label backfill); pre-convention revisor seeds carry the
`revision` label retroactively (55 seeds, 2026-09-03).

## Considered options

Rejected: note-only supersession (the run's workaround) — leaves stale open
seeds as perpetual deadwork in `sd ready`. Rejected: automatic closing by
the analyzer — mixing the drop-decision with the replace-decision caused
exactly the near-duplicate confusion the tracker check (816d44e67) fixed;
one relation per actor keeps both auditable in the journal.

## Consequences

Closing is destructive-ish (a closed seed can be reopened, but triage stops
seeing it), so the rule is deliberately conservative: when unsure, close
nothing and record the suspicion for the human gate. The human approve gate
remains the final control on every supersession.
