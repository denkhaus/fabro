# ADR-0040: Branch model — denkhaus as the integration trunk, the merger as the central integration instance

Date: 2026-11-08
Status: accepted (supersedes the ref-keeping half of ADR-0024's upstream posture; ADR-0024's intake freeze stands)

## Context

The repository carried three long-lived branch classes: `denkhaus` (the line, written directly by
sessions behind a mechanical push gate and by the run publisher for run branches), `main` (the
upstream ghost — GitHub's default branch, 2432 commits behind, intake frozen since ADR-0024), and
accumulated debris (`denkhaus-0`, five `arch/*` branches, numeric-prefix leftovers, one `arc/run`
stray). The merger already exists as the integration authority for automation work: run branches
land as pull requests and the merger integrates merge-enabled runs through its door (ADR-0039
stage E). Session work bypassed it: sessions pushed the trunk directly, gated only on line
quietness — which zombie claims and in-flight passes repeatedly wedged (observed 2026-11-08: the
disk-outage hotfix waited on stale claims).

## Decision

1. **`denkhaus` is the integration trunk.** Nobody writes it in the normal course of work.
2. **Session work lands as fix/feature branches.** A branch opens a PR onto `denkhaus` carrying
   the label `merge:enabled`; the merger's sweep treats labelled PRs exactly like merge-enabled
   run PRs — same door, same verification, same smoke of the integrated tree. The label is the
   public contract (it mirrors the adjudicator's `merge=enabled` verdict vocabulary).
3. **The direct trunk push is reserved for incident restore** — line down, gate unblock, the
   class exercised on 2026-11-08. Normal work accepts merger-cadence latency; emergencies keep
   immediacy.
4. **`main` is deleted** after the GitHub default branch moves to `denkhaus`. The frozen-base
   identity does not depend on the ref: it lives in version strings, release tags, and
   `vendor/PINS.toml`. An upstream reopen (ADR-0024's re-open condition) re-fetches upstream
   regardless.
5. **All dead refs go in the same ceremony**: `denkhaus-0` (zero references), `arch/*`,
   the numeric-prefix leftovers, the `arc/run` stray. The branch listing afterwards shows only
   `denkhaus`, `fabro/run/*`, and active work.

## Consequences

- The push gate's meaning narrows to the incident reservation; the iterate skill's push rules
  follow this ADR.
- Hotfix latency for normal work is bounded by the merger's cadence; the resolve pass's missing
  push capability (denkhaus/work#1192) becomes more load-bearing and should land soon.
- Branch admission is label-gated — a mislabelled PR is simply ignored (fail-closed), never
  auto-merged.
- GitHub's default-branch switch changes where new clones and PR targets point; run PRs already
  target the trunk.
