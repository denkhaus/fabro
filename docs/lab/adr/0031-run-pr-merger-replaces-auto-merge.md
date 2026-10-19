# ADR-0031: Run-PR merger replaces auto-merge

- Status: accepted (2026-10-19)
- Deciders: user (grill, all pre-screen recommendations accepted)
- Supersedes: run-PR auto-merge (fabro-c14e build form (ii); the
  discarded fabro-a19a throttling approach)

## Context

Run PRs (branches `fabro/run/*`) landed through GitHub auto-merge:
first-green-wins, no ordering. Two simultaneously-green PRs conflict on
the shared tracker/review files; a stale merge ref merges against an old
line; the fabro-006d TOCTOU race (line moves between a pass's claim
fetch and its start checks) fires whenever merges land near the :00/:30
ticks. The session performed this sequencing manually on 2026-10-19
(PR #441/#439: rebase, duplicate-review resolution, merge-wait).

## Decision

1. **Auto-merge is OFF, platform-wide.** The PR pipeline's auto-merge
   enablement is a no-op with a deprecation warning; the
   `[run.pull_request] auto_merge` key is inert (kept for config
   compatibility, documented deprecated). Nothing but the merger lands
   run PRs.
2. **A deterministic merger arms the existing staleness supervisor**
   (fabro-895d machinery — DB-driven candidates from platform records,
   update-branch with strikes, closed-wins union merge for tracker-only
   conflicts, >24h retirement). No agent, no LLM, no new capability
   grant (ADR-0019 unaffected).
3. **Merge windows**: the merger merges only in minutes :10–:25 and
   :40–:55 of each hour. Ticks fire :00/:30 and claims land ~:00–:04, so
   line movement stays outside claim windows — the fabro-006d race is
   driven to near zero without throttling ticks. Branch updates and
   retirements are not window-gated (they do not move the line).
4. **Sequencing**: base-first — mergeable-clean candidates sort by PR
   number (oldest first) and merge in that order within a pass.
5. **Duplicate close (exact predicate)**: a DIRTY run PR whose
   changed-file set is identical to a PR the same pass just merged
   closes immediately citing the merge sha — the fast path over the
   3-strikes retirement for the same-files twin class (observed:
   duplicate review/marker files from two revisor passes on one target).
6. **Fail-closed**: a pass error warns and retries on the next scan
   (15 min); a parked or erroring merger merges nothing. No fallback to
   any other landing path exists (auto-merge is off).

## Consequences

- Re-arming the loop/revisor schedules is safe with respect to the
  claim race: their PRs land only inside windows.
- Run PRs stay open a few minutes longer than under auto-merge (window
  wait); that is the price of ordering.
- `gh pr merge` by a human remains possible (the merger adds a policy,
  not a permission wall); the session keeps its emergency path.
