# Revision — run 01M2YPG3YHRNTWSCAS9T6EN7SQ

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2YPG3YHRNTWSCAS9T6EN7SQ.md
- seeds filed: none — zero balance credit this pass (0 stale/superseded closes)
- balance: 0 non-exempt seeds filed / 0 — no credit this pass
- basis: run 01M2YPG3YHRNTWSCAS9T6EN7SQ, workflow version be71015a49744c64572bb5eada073fed11dbd7df4594d32d29988bf727e26454, commit 9a866d4b35d6f9c7bd08c1d2fd16e6f77323d200
- revised_at_commit: 9a866d4b35d6f9c7bd08c1d2fd16e6f77323d200 (ADR-0015: engine drift signal for later judgement)

## Findings

### Park externally-blocked seeds fabro-af22 and fabro-23d4 (tracker hygiene)
Concrete change: mark both seeds blocked-external (label or dependency) using infeasibility facts already in run 01M2YPG3YHRNTWSCAS9T6EN7SQ's planner journal, mirroring the one-off fabro-a285 park. Expected effect: saves ~$0.15–0.20 and ~1.5 min per develop lap.
Not filed — same themes already open in the overflow ledger:
- overflow-dup: Park upstream-PR-gated seeds mechanically in the planner preflight (open in 01M2VVFMSQXHK0E3SNV2GNTP43.md)
- overflow-dup: Flag seeds whose change target lives outside the repo at preclaim/intake (open in 01M2YMRJQZMC6SVTQJ3JC49CWC.md, covers fabro-23d4 parking)

### Make destructive ml subcommands safe by default (ml prune dry-run unless --yes)
Concrete change: guard destructive `ml` subcommands (wrapper or upstream) so `ml prune` defaults to dry-run and requires explicit `--yes`, same shape as `sd close`'s mandatory `--reason`. Basis: bare `ml prune` soft-archived ~30 records this run; recovery needed `git checkout -- .mulch/` plus manual re-application. Expected effect: eliminates a silent mass-destruction class for every implementer pass touching lessons. Dedupe verified: no open seed covers destructive-subcommand guarding (fabro-b94d, fabro-8d81 cover other mulch gaps).
- overflow: Make destructive ml subcommands safe by default (ml prune is dry-run unless --yes) — guard destructive `ml` subcommands via wrapper or upstream change so `ml prune` defaults to dry-run and requires explicit `--yes`; effect: eliminates a silent mass-destruction class for every implementer pass that touches lessons.

### Seed-intake lint arm: 'extend the <fabro-id> mechanism' citations must reference landed seeds
Concrete change: add an arm to open seed fabro-3839's lint surface — a body phrase 'extend the <fabro-id> mechanism' fails lint unless that seed is closed. Basis: claimed seed fabro-7aac cited 'extend the fabro-534e mechanism' while fabro-534e was open; the planner burned probes (transcript seq 94–97) and rewrote via `sd update --description`. Expected effect: no planner re-hits an unlanded-mechanism contradiction; ~1 min saved per occurrence. Dedupe verified: orthogonal to fabro-3839's current arms and to fabro-d20f/fabro-41de.
- overflow: Seed-intake lint arm: 'extend the <fabro-id> mechanism' citations must reference landed seeds — extend fabro-3839's filed-seed lint so a body phrase 'extend the <fabro-id> mechanism' fails lint unless that seed is closed; effect: no planner ever re-hits an unlanded-mechanism contradiction (~1 min saved per occurrence).
