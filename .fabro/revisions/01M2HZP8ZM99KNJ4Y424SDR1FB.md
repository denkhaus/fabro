# Revision — run 01M2HZP8ZM99KNJ4Y424SDR1FB

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2HZP8ZM99KNJ4Y424SDR1FB.md
- seeds filed: fabro-cf31 — PR fallback title: derive from the claimed seed title, not the truncated goal
- basis: run 01M2HZP8ZM99KNJ4Y424SDR1FB, workflow version edb25cfabdde6b4bd357b60e5e4217d970140b6ed530094a1d148ecb806c72dc, commit 908aba6db264d1c12d3527341bf4cc5f7d77ff02
- revised_at_commit: 908aba6db264d1c12d3527341bf4cc5f7d77ff02 (ADR-0015: engine drift signal for later judgement)

## Findings

### PR fallback title derives from truncated goal, not seed title — filed fabro-cf31

`fallback_pr_title` in `lib/components/fabro-workflow/src/pipeline/pull_request.rs` uses `pr_title_from_goal` (goal first line), so this run's merged PR #150 carried the generic truncated goal instead of the claimed seed's title (fabro-3e48). Fix: prefer `current_seed_title` on the fallback path, keep goal-derived line only when no seed title exists. Evidence: worker log 07:34:44 'retry was not JSON; salvaging prose body with deterministic title'. Not a duplicate: fabro-41b1 covers strictness, fabro-c7f7/fabro-2b7a added the fallback mechanism itself; no seed names the fallback title source.
