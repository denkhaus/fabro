Done. Simplify pass on PR #547 is complete — reviewed by three parallel sub-agents, fixes applied, and the existing PR updated in place (no new branch/PR).

## What was fixed

**High-impact (flagged by all three reviewers):**
- **Redact once at the sink boundary** — `RunEventSink` now redacts at the `RedactSecrets` node (skipped entirely when no secrets are registered) instead of threading `Option<SecretRedactor>` into every leaf. This removes a full serialize→content-scan→reparse pass that ran **per event per fanout leaf** on the streaming hot path, even for runs with no secrets, and deletes the fail-open `append_run_event_with_redactor` trait method whose default silently dropped the redactor.
- **Collapsed ~13 `foo`/`foo_with_redactor`/`foo_inner` twin functions** across fabro-sandbox, fabro-workflow, and fabro-cli into single signatures taking `&SecretRedactor` (an empty redactor is an identity).

**Smaller cleanups:** dropped the redundant `run_id` param from `redacted_run_event`; folded `HookRunner::new_with_secrets` into `new`; replaced the hand-written `decision_label` map with `strum::IntoStaticStr` per repo policy; replaced two copy-pasted 5-boolean reqwest log blocks with `e.without_url()`; added `HookType::interp_strings()` as the single source of truth for interpolatable hook fields; avoided a per-header `Vec` allocation in `first_secret_name`.

**Also:** merged `origin/main` into the branch (resolving the `start.rs` overlap with #545), so the PR diff now shows only its real delta. Net: **−107 lines** on top of that merge.

**Skipped** (noted in the PR comment): the event-property allowlist colocation, the env+secrets resolver-context bundle, the `RunSecrets` ownership bundle, the fabro-redact snapshot micro-opt (main-side code), `BoxFuture` (new dep for a type alias), and per-definition secret-name precompute.

**Verified:** workspace build, nextest on all touched crates (1200+ workflow tests, 1300+ cli/acp/types tests — all pass; one unrelated flaky socket test passes standalone), nightly fmt and clippy `-D warnings` clean.

**PR updated:** pushed `c5cf4b7f9` (merge) + `6d6a1d56a` (simplify) to the existing branch, [commented](https://github.com/fabro-sh/fabro/pull/547#issuecomment-4906844207), and labeled `simplify:claude-fable-5`.