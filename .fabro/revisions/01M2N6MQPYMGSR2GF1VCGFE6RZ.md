# Revision — run 01M2N6MQPYMGSR2GF1VCGFE6RZ

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2N6MQPYMGSR2GF1VCGFE6RZ.md
- seeds filed:
  - fabro-d89a — Evidence capture: emit per-criterion check outputs for implementer runs, not just verification-only runs
  - fabro-260c — Engine preamble renderer: render multi-line context values outside the pipe-table instead of collapsing to ' / '
  - fabro-17df — Implementer lesson-capture: require ml record for reusable patterns, not only near-miss lessons
  - fabro-3805 — Seed-authoring lint: verification commands in seed bodies must be runnable within the tool's actual scan scope
- basis: run 01M2N6MQPYMGSR2GF1VCGFE6RZ, workflow version e1bda2f455a07c0d501ab040c95e4344d05b2f486e5a471db6e4c56668ad1c61, commit a69aca3b7d8a33700917805a3666fa2e100bec78
- revised_at_commit: a69aca3b7d8a33700917805a3666fa2e100bec78 (ADR-0015: engine drift signal for later judgement)

## Findings

### Evidence capture: emit per-criterion check outputs for implementer runs, not just verification-only runs
- filed: fabro-d89a
- Change: extend `.fabro/workflows/develop/scripts/evidence.nu` to include the transcript of the implementer's per-criterion checks in every capture. Effect: reviewer approves from context; removes duplicated re-verification (~2 shell calls, ~30-45s, ~$0.02-0.04 per run). Extends open fabro-f759 (verification-only scope) — superset, not a duplicate; distinct from closed fabro-50c9.

### Engine preamble renderer: render multi-line context values outside the pipe-table instead of collapsing to ' / '
- filed: fabro-260c
- Change: engine preamble renderer (lib/) — emit multi-line values (at least `current_seed_brief`) as a bullet list below the table or preserve newlines in cells. Effect: bulleted briefs actually reach downstream stages. Closed fabro-f677 chose the collapse as its fix (that collapse IS this bug); open fabro-9e49 covers planner-side emission only — cross-referenced, not superseded.

### Implementer lesson-capture: require ml record for reusable patterns, not only near-miss lessons
- filed: fabro-17df
- Change: `.fabro/workflows/develop/prompts/implementer.md` lesson-capture section — an observation naming a reusable pattern requires `ml record` + mx-id. Effect: durable expertise capture via `ml prime --files`. Extends open fabro-ee2c (trigger currently limited to near-miss tool lessons) — cross-referenced, not superseded.

### Seed-authoring lint: verification commands in seed bodies must be runnable within the tool's actual scan scope
- filed: fabro-3805
- Change: extend the seed-contradiction lint of open fabro-7773 with validation that seed proof recipes are executable within the referenced tool's scan scope. Effect: implementers stop burning cycles on impossible proof recipes; orthogonal to open fabro-7bb3 (path presence).
