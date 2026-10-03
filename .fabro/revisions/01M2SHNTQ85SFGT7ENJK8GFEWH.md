# Revision — run 01M2SHNTQ85SFGT7ENJK8GFEWH

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2SHNTQ85SFGT7ENJK8GFEWH.md
- seeds filed: fabro-ead4 (preflight superseded-close for non-top duplicate candidates + verdict legend), fabro-d856 (rust-style-guide hard gate must read via shell, not read_file), fabro-a211 (document routing-field activation contract for file schemas + optional prompt-lint warning)
- basis: run 01M2SHNTQ85SFGT7ENJK8GFEWH, workflow version e532acb40fcbc81d393a7f4e50e20c8568b5d52038eeb81396b978e2bbef2927, commit f3819eadf88dc51df90af67b25763492c01061e4
- revised_at_commit: f3819eadf88dc51df90af67b25763492c01061e4 (ADR-0015: engine drift signal for later judgement)

## Findings

### Planner-preflight closure arm misses non-top duplicate candidates
- filed: fabro-ead4
- Change: in `.fabro/workflows/develop/scripts/planner-preflight.nu`, apply the existing closure-note + superseded-close to ANY candidate with an unambiguous duplicate verdict (currently only the TOP candidate), and emit a one-line verdict legend in `output.preflight`. Evidence: run returned `closed: {"seed": null}` despite an unambiguous duplicate verdict (fabro-90ae, sha 4f8a254, via #232) for candidate #3; fabro-90ae remains open and is re-reported every run. Expected effect: mechanical closure of recurring duplicates and one saved tool round (planner read the 80-line script header, seq 40–42, to learn the verdict vocabulary).

### Hard-gate prompts teach the wrong affordance on an fs_hide-bound path
- filed: fabro-d856
- Change: in `.fabro/workflows/develop/prompts/implementer.md` (line 24) and `reviewer.md` (line 25, RUST STANDARDS AXIS), mandate shell reads (`sed -n`/`cat`) of `.fabro/skills/rust-style-guide/SKILL.md` instead of `read_file`, which the nodes' `fs_hide=".fabro/**"` always denies. Evidence: implementer denied on its literal first action (seq 101, error_kind denied), recovered via shell (seq 108); fires twice per Rust run. Expected effect: −2 wasted error/recovery rounds per Rust run.

### Routing-field activation contract undocumented for schema authors
- filed: fabro-a211
- Change: add a paragraph to the workflow-asset schema-authoring guidance stating that any `@schemas/*.json` payload containing a routing-named top-level field (e.g. `context_updates`) is routing-active since fabro-0a4c; optionally add a `prompt-lint.nu` warning for such schemas. Evidence: implementer flagged MATERIAL SEMANTIC RISK in `implementation_summary` after #235. Expected effect: authors stop silently opting file schemas into routing semantics.
