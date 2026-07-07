Review complete and fixes applied. Summary:

## Review verdict
The implement-stage change was fundamentally sound: all three agents confirmed `snakeify` was removed with zero remaining references repo-wide, `kebabify` reuse is correct, the new help text matches the on-disk kebab-case workflow directory convention, the per-keystroke `kebabify` cost is identical to what it replaced, and the `changeField` test helper follows established suite conventions with proper renderer cleanup.

## Fixed (3 issues)
1. **Dead export** — `kebabify` in `automation-form.tsx:119` was exported with no importers anywhere (same smell as the just-deleted `snakeify`). Made it module-private.
2. **Route mount for pure-function coverage** (flagged by two agents) — the "kebab-cases workflow name fallback" test built the full memory-router + mocked-hooks render just to assert the fallback branch of `automationFormValuesFromRun`. The prefill *wiring* is already covered by the adjacent "pre-populates from run" test, so it's now a direct call to the exported pure function (test runtime dropped from ~9ms to 0.07ms).
3. **Locator duplication** — `renderer.root.findByProps({ "aria-label": label })` appeared 3× across `fieldValue`/`changeField`/`switchChecked`; extracted a shared one-line `byLabel` helper.

## Noted, intentionally skipped
- **Per-keystroke normalization strips trailing dashes/spaces while typing** (`kebabify("patch-")` → `"patch"`), so multi-word slugs must be pasted or edited mid-string. This is pre-existing, identical to the Automation slug field, and the plan explicitly locks the live-normalization design — worth a separate follow-up (normalize on blur) covering both fields.
- **`automations.tsx:58-74`** — `slugIconMap`/`slugColorMap` are keyed by snake_case workflow slugs (`fix_build`, …) that kebab-created automations will never match, falling back to the default icon/color. Cosmetic, pre-existing, and outside this plan's "no other files" scope.
- Hoisting `changeField` into `test-utils.tsx` — only one consumer today; four other test files inline the pattern, so a shared helper is a follow-up, not a requirement.

## Verification
- `bun run typecheck` — clean
- `bun test` — 652 pass / 0 fail across 82 files