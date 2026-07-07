Implemented the frontend-only kebab-case workflow slug fix.

Changes made:
- `apps/fabro-web/app/components/automation-form.tsx`
  - Workflow slug input now uses `kebabify()` instead of `snakeify()`.
  - “create from run” fallback now kebab-cases workflow names.
  - Updated help text and placeholder to dash-separated examples.
  - Removed unused `snakeify()` helper.

- `apps/fabro-web/app/routes/automations-new.test.tsx`
  - Updated from-run expectation to `fix-ci`.
  - Added regression coverage for:
    - `patch-cves` staying `patch-cves`
    - `Patch CVEs` becoming `patch-cves`
    - from-run fallback name becoming `patch-cves`

TDD/verification:
- Red confirmed after test update:
  - `cd apps/fabro-web && bun test --isolate app/routes/automations-new.test.tsx`
  - Failed as expected with `patch_cves`.
- Green verified:
  - `cd apps/fabro-web && bun test --isolate app/routes/automations-new.test.tsx`
  - `cd apps/fabro-web && bun run typecheck`
  - `cd apps/fabro-web && bun test`
  - `git diff --check`

Also confirmed:
- No `snakeify` references remain under `apps/fabro-web/app`.
- No backend/Rust files were changed.