# Journals are per-run JSONL streams; work and publish outcomes are separate

## Status

Accepted (2026-08-24). Grounded in runs 01M0SS23MJ, 01M0T2GW, 01M0T9B7T6.

## Context

The stage journal (fabro-176b) originally wrote one JSON file per stage
execution: `.fabro/journal/<node>@<visit>.json`. Runs share a base branch,
so the visit glob counted across runs (run 01M0T2GW wrote `planner@4.json`
for its first planner visit) and two runs from one base write colliding
names — a guaranteed auto-PR conflict on journal files, not code. With
autonomous PR merge-back every run would have added more unattributable
files.

Separately, three runs in a row ended `failed(publish_failed)` on a GitHub
403 after the graph finished green: seeds closed, gate green, review
approved, run branch pushed. The single red status buried every green
signal and lied about the dev loop.

## Decision

1. **Journal stream**: one `.fabro/journal/<run_id>.jsonl` file per run,
   one JSON line per stage completion (`fabro-journal-v1` stays the line
   shape). Visits are run-local (same-node lines in THIS file + 1).
   Appends repair a missing trailing newline before writing (a crashed
   append must not glue two entries into one unparseable line); consumers
   tolerate one unparseable trailing line. Separate run ids never collide.
   Legacy `<node>@<visit>.json` files are not migrated and never written
   again.

2. **Work outcome ≠ publish outcome**: a publish failure after a green
   graph is not a dev-loop failure. The taxonomy must separate them
   (target: `succeeded_with_publish_failure` — platform-side, seed
   pending); the run branch remains the durability path and the remediation
   surface (manual merge or manual PR from `fabro/run/<id>`). A preflight
   credential probe (token validated against the GitHub REST API before
   the run starts) turns the 8-minute-late 403 into a 1-second refusal.

## Consequences

- Auto-PR merge-back is journal-collision-free; parallel runs from one
  base stay independent.
- The improve workflow reads one ordered stream per run instead of
  globbing N files.
- Run 01M0T9B7T6 validated the format live: single `<run_id>.jsonl`,
  7 lines, run-local visits, conflict-free merge.
- The publish-side items (outcome taxonomy, preflight probe, publish
  retry classification) are platform work, tracked as platform seeds —
  they do not block this decision.
