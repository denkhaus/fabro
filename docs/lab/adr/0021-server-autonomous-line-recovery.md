# ADR-0021: Server-autonomous line recovery via fixed recheck probes

- Status: Accepted
- Date: 2026-09-14
- Deciders: user (decision in chat), agent (analysis + implementation)
- Seeds: fabro-986b (decision), fabro-0e11 (stall prerequisite), fabro-183f (retry-burn facet)
- Supersedes: none (extends the fail-closed line posture of ADR-0015/0018)

## Context

The autonomous line (conductor automation on the production server) died
on 2026-09-13 when the zai 5-hour usage window closed mid-pass
(run 01M2E7VZYX8V). Forensics showed a three-layer stack, not a single
defect:

1. The usage-window 429 burned five stage retries against a window hours
   away (every attempt re-sent the full stage prompt, fabro-183f) and the
   exhaustion path surfaced as a deterministic goal-gate failure, hiding
   the quota cause from every downstream consumer.
2. The conductor's legal blocking `fabro_run_wait` (60 min by design)
   emitted no events, so the default 1800 s stall watchdog killed the
   pass mid-wait (fabro-0e11).
3. Every recovery path ended in an EXTERNAL flip: terminal-failed runs
   stay dead, the schedule breaker latches after three same-signature
   failures and never re-enables itself, and revival required a human.

The engine already had most of the building blocks: rate-limit reset
prose detection (fabro-a3d8, fabro-0607), a SoftStop park mapping that
keeps runs resumable, an event-driven stall watchdog with a graph-level
`stall_timeout` attribute, and the serialized scheduled-fire path with
`on_overlap=skip`.

## Decision

1. **Fixed recheck cadence, never prose-parsed backoff.** The recovery
   timing is a fixed 10-minute interval (user decision 2026-09-14).
   Provider prose reset times are never parsed for timing: zai sends
   offset-less wallclocks, and a fixed probe is simple, bounded, and
   observable. Only the PRESENCE of an announced usage window is used
   (to distinguish a usage-window 429 from an ordinary short one).
2. **Quota parks park, they do not burn.** A usage-window rate limit
   fails the stage non-retryably and the run ends as a resumable
   `SoftStop` carrying the `api_transient|<provider>|rate_limit`
   signature. It does not feed the failure-routing loop (which ended as
   the deterministic goal-gate error).
3. **Quota parks are breaker-exempt.** They count toward neither the
   latch nor a reset; the breaker stays armed for real (non-quota)
   defects.
4. **Recheck probes fire through the normal scheduled-fire path** — same
   serialization (`on_overlap=skip`), same materialization, no new fire
   lane. Probe state is derived in-memory (per-automation last-probe
   gate); a server restart may fire one early probe, which is harmless.
5. **Stall budget covers the legal wait.** The loop graphs pin
   `stall_timeout="63m"` so a legal 60-minute wait (plus slack) cannot
   trip the default 1800 s watchdog. The watchdog stays armed for
   genuine agent silence.
6. **Run-recovery supervisor (server-side resume of parked runs) is
   DEFERRED, not rejected** (option A in fabro-986b). Recheck probes
   re-plan instead of resuming; revisit if redo cost becomes painful.
7. **Fork-file policy for the implementation** (user directive
   2026-09-14): the feature logic lives in fork-only files
   (`fabro-workflow/src/fork_line_recovery.rs`,
   `fabro-server/src/server/fork_line_recovery.rs`), wired through
   minimal one-line seams, with presence pins in fork-only test files
   (`fork_line_recovery_tests.rs`) plus a breaker-exemption pin in the
   scheduler test module and rows in the merge-upstream touchpoints
   list. An upstream merge cannot conflict the files away; a dropped
   seam reds the gate.

## Consequences

- A quota hard-cut no longer takes the line down for good: probes every
  10 minutes until a pass survives the reopened window. No external
  trigger, no manual re-enable.
- A quota park leaves a resumable run behind; today we do not resume it
  (the next pass re-plans the seed). The parked run is evidence and a
  future supervisor's entry point.
- Each probe costs one pass start up to the first LLM call (it parks
  again immediately while the window is closed). Bounded by the 10-minute
  cadence; visible in server logs as `line recheck probe firing`.
- Non-quota failures (real defects) still trip the breaker exactly as
  before — fail-closed posture for actual breakage is unchanged.
