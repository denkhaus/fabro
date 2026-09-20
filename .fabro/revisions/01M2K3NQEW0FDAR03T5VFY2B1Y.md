# Revision — run 01M2K3NQEW0FDAR03T5VFY2B1Y

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2K3NQEW0FDAR03T5VFY2B1Y.md
- seeds filed: fabro-2f70 — Fix verify.nu test detection for inline #[cfg(test)] modules; fabro-a03e — Route implementer risk flags into reviewer view and add the retry-jump test case
- basis: run 01M2K3NQEW0FDAR03T5VFY2B1Y, workflow version ab2ee24a456a34dfd349075bbbf8a38de2d071f30582720402cc4fedd93a2170, commit 34206de978fc1bcc3bb1b2b89b6080c0e59e679b
- revised_at_commit: 34206de978fc1bcc3bb1b2b89b6080c0e59e679b (ADR-0015: engine drift signal for later judgement)

## Findings

### Fix verify.nu test detection for inline #[cfg(test)] modules
- filed: fabro-2f70
- `scripts/verify.nu` `is-test-file` only matches `/tests/` dirs, `*_tests.rs`, `/tests.rs`; inline `#[cfg(test)]` test modules (project convention) leave the crate un-flagged test-file-touched, degrading verify to compile-check. Change: also flag crates whose diffs touch `#[cfg(test)]` regions. Expected effect: implementer pre-gate test signal fires on every Rust seed using inline test modules. Distinct from fabro-7f58 and fabro-6e7f.

### Route implementer risk flags into reviewer view and add the retry-jump test case
- filed: fabro-a03e
- Implementer's flagged retry-semantics risk (quarantine reverts half-edits and retries from last checkpoint) never reached the reviewer: `journal` absent from reviewer preamble_allow_keys and `implementation_summary` didn't repeat it. Changes: require material semantic-risk observations in `implementation_summary` (`.fabro/workflows/develop/prompts/implementer.md` schema lines 36/145), and add a 4th retry-jump case to the test table in `lib/components/fabro-workflow/src/lifecycle/git.rs`. Expected effect: flagged risks can't escape review; PR #153's feature gets its missing test.
