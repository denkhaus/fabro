Goal: # Plan: automation "Workflow slug" field must use kebab-case (dashes), not snake_case

## Summary

The web UI's New/Edit Automation form silently rewrites the **Workflow slug**
field to snake_case on every keystroke (`patch-cves` → `patch_cves`). The rest
of the system uses kebab-case (dashes) for workflow slugs, and workflow
directories on disk are dash-separated (`.fabro/workflows/patch-cves/`). So a
snake_cased selector points at a non-existent directory, and scheduled
automations targeting any multi-word workflow **silently never fire**.

Fix: change the Workflow slug field (and the "create from run" fallback) to use
the form's existing `kebabify()` helper instead of `snakeify()`, update the
field copy, remove the now-unused `snakeify()`, and update the test. This is a
**frontend-only** change; the backend already accepts dashes and needs no edit.

## Root cause (verified, with file:line)

All in `apps/fabro-web/app/components/automation-form.tsx`:

- `snakeify()` (lines 127-133) — `.toLowerCase().replace(/[^a-z0-9_]+/g, "_")…`
  converts any dash to an underscore.
- The Workflow slug input (lines 302-311) applies it on every keystroke:
  ```tsx
  // line 303 help text:
  help="Snake-case identifier used in the workflow file name (e.g. fix_build.fabro)."
  // line 310:
  onChange={(e) => patch({ workflow: snakeify(e.target.value) })}
  // line 311:
  placeholder="fix_build"
  ```
- The "create automation from run" prefill (line 89) also snake-cases the
  fallback name:
  ```tsx
  workflow: run.workflow.slug?.trim() || snakeify(workflowName),
  ```
- `kebabify()` (lines 119-126) already exists and is used for the automation's
  own id field (lines 85, 214, 220). It is the correct normalizer to reuse.

`snakeify` is referenced in exactly two places (lines 89 and 310); once both
move to `kebabify`, the `snakeify` definition (lines 127-133) is dead code.

## Why kebab-case is correct (justification)

The snake_case rule is a UI-only outlier. Every other layer treats workflow
slugs as kebab-case:

- Backend `validate_workflow_selector` (`lib/crates/fabro-automation/src/model.rs:366`)
  allows both `-` and `_`, so it will accept a dashed value with no change.
- Real workflow directories are dash-separated: `patch-cves`, `gh-triage`,
  `implement-plan`, `card-game`, `implement-issue`, etc. Only single-word ones
  (`smoke`, `hello`) survive `snakeify` unchanged.
- `workflow_slug_from_path` (`lib/crates/fabro-config/src/project.rs:111`)
  preserves dashes, and `lib/crates/fabro-workflow/src/run_options.rs:31`
  documents the slug as the **literal workflow directory name**.
- `AutomationId` (`lib/crates/fabro-automation/src/id.rs`) actually **forbids
  underscores** — dashes only.

So kebab is the established convention; the form is simply wrong. No backend
change is needed or wanted.

### Two distinct names — do not conflate (important)

A workflow has two separate identifiers, and only one is relevant here:

- **Workflow slug** — derived from the *directory* name, kebab-case
  (e.g. `patch-cves` from `.fabro/workflows/patch-cves/`). This is what the
  automation `workflow` selector resolves against, and what this fix must
  produce. (Seen as `workflow_slug: "patch-cves"` on runs.)
- **Graph name** — the identifier in the DOT source, `digraph PatchCves { … }`,
  typically CamelCase. This is internal graph identity (`name: "PatchCves"` on
  runs); it is **not** used to resolve the automation target.

The Workflow slug field must be the kebab directory slug. Do **not** change it
toward the CamelCase graph name, and do not touch graph names anywhere. This
also explains the original mistake: the author treated the file/dir name as
snake_case when it is actually kebab-case.

## Failure mode this fixes (for context)

1. User types `patch-cves` → form stores `patch_cves`.
2. Server stores it verbatim (no normalization; validator accepts underscores).
3. On each cron tick, `automation_materializer` resolves the selector literally
   → looks for `.fabro/workflows/patch_cves/` → `WorkflowNotFound`
   (`lib/crates/fabro-server/src/automation_materializer.rs` ~219-229).
4. `lib/crates/fabro-server/src/server/automation_scheduler.rs:221-239` logs
   `"Failed to materialize scheduled automation run"` and returns — no run
   created, waits for the next tick. The automation appears to do nothing.

## Where it was introduced (provenance, informational)

- `snakeify` + the Workflow slug field + "Snake-case identifier…" help text:
  commit `2e85a1ec4` "Add New Automation form and refresh Secrets form layout"
  (2026-05-24). Present from the form's first draft; no commit message explains
  why the workflow field is snake-cased.
- Carried into the shared component: `87516c25c` (2026-05-28).
- Prefill fallback snake-cases the name: `a65473f21` / PR #454 (2026-05-29).
- Backend validator that (correctly) allows dashes: `e13de9faa` / PR #428.

## Implementation steps

Frontend only. Use red/green TDD.

1. **Update the test first** — `apps/fabro-web/app/routes/automations-new.test.tsx`:
   - The assertion at ~line 278 currently expects the snake output
     `expect(fieldValue(renderer, "Workflow slug")).toBe("fix_ci")`. Change the
     expected value to the kebab form (`"fix-ci"`) to match the new behavior.
   - Add a regression assertion: typing `patch-cves` into the "Workflow slug"
     field leaves it as `patch-cves` (dashes preserved, NOT converted to
     underscores). Also confirm `Patch CVEs` → `patch-cves`.
   - Run the test and confirm it fails against current code (red).

2. **Fix the field** — `apps/fabro-web/app/components/automation-form.tsx`:
   - Line 310: `onChange={(e) => patch({ workflow: snakeify(e.target.value) })}`
     → use `kebabify(e.target.value)`.
   - Line 89: `run.workflow.slug?.trim() || snakeify(workflowName)`
     → `run.workflow.slug?.trim() || kebabify(workflowName)`.
   - Line 303 help text → describe dash-separated, e.g.
     `"Dash-separated identifier matching the workflow directory name (e.g. patch-cves)."`
   - Line 311 placeholder `"fix_build"` → `"patch-cves"` (or `"fix-build"`).
   - Remove the now-unused `snakeify()` function (lines 127-133). Keep
     `kebabify()` (still used for the id field and now the workflow field).
   - Verify no references remain: `grep -rn "snakeify" apps/fabro-web/app`
     should return nothing.

3. **Verify** in `apps/fabro-web`:
   - `bun run typecheck` passes.
   - `bun test` passes (the updated + new assertions go green).

## Files to touch

- `apps/fabro-web/app/components/automation-form.tsx` — the fix (lines 89, 303,
  310, 311; remove 127-133).
- `apps/fabro-web/app/routes/automations-new.test.tsx` — update the snake
  assertion (~278) and add the dash-preservation regression test.
- No other files. `automations-new.tsx` / `automations-edit.tsx` submit
  `values.workflow.trim()` unchanged and need no edit.

## Acceptance criteria

- Typing `patch-cves` in "Workflow slug" yields `patch-cves` (not `patch_cves`).
- Typing `Patch CVEs` yields `patch-cves`.
- The "create from run" prefill uses the run's dashed slug when present and
  kebab-cases the fallback name.
- Help text and placeholder reflect dash-separated slugs.
- No `snakeify` references remain (dead code removed).
- `bun run typecheck` and `bun test` pass in `apps/fabro-web`.
- No backend/Rust changes.

## Out of scope

- **Backend changes.** `validate_workflow_selector` already accepts dashes;
  leave it. Do not add snake↔dash conversion on the server.
- **`kebabify()` itself** — it is correct for the id field; do not modify it.
- **Migrating already-created automations** whose stored `workflow` is already
  snake_cased. That is an operational fix (edit the automation via
  `PUT /api/v1/automations/{id}` with the corrected dashed `workflow`), not part
  of this code change.
- The unrelated sandbox `glob` consistency fix (tracked in a separate plan).

## Design decision (stated so it isn't re-litigated)

Keep normalizing the field — do not make it freeform. Normalize to **kebab-case**
(reusing the existing `kebabify`) so it matches the id field's behavior, the
backend, and the on-disk workflow directory convention.

## Conventions to follow

- Match the surrounding TypeScript/React style in `apps/fabro-web`.
- This is an `onChange`/string-helper change only — no new React effects (the
  repo avoids direct `useEffect`; none is needed here).
- Keep the change minimal and focused; do not refactor unrelated form code.


## Completed stages
- **toolchain**: succeeded
  - Script: `command -v cargo >/dev/null || { curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y && sudo ln -sf $HOME/.cargo/bin/* /usr/local/bin/; }; cargo --version 2>&1`
  - Output:
    ```
    cargo 1.95.0 (f2d3ce0bd 2026-03-21)
    ```
- **preflight_compile**: succeeded
  - Script: `cargo check -q --workspace 2>&1`
  - Output: (empty)
- **preflight_lint**: succeeded
  - Script: `cargo +nightly-2026-04-14 clippy -q --workspace --all-targets -- -D warnings 2>&1`
  - Output: (empty)


Read the plan file referenced in the goal and implement every step. Make all the code changes described in the plan. Use red/green TDD.