# Handoff: retention window + open threads (2026-11-20 session end)

Read this FIRST, then delete it once consumed (ADR-0036: handoffs are
transport, not history).

## TIME-CRITICAL: tonight ~00:55 UTC

The scheduled retention pass (24h cadence, ungated by the archive
filter — that gate is work#1514, NOT yet built) deletes ALL terminal
runs older than 7 days that pass the #1299 triage gate. The dry-run
counted 71 candidates (the Nov 6-8 batch, mostly archived, gate
allowing).

The operator decided (ADR-0048): runs must NOT be auto-deleted
wholesale — the pass becomes archive-gated (work#1514, lane:develop,
not yet built).

FIRST DUTY BEFORE 00:55 UTC: apply the tofu stopper.

1. Warm the gopass store: a BLOCKING `gopass show` on the state
   passphrase path (fabro-tofu `envs/state-secret`) — do it right
   before the apply, the TTL is ~30 minutes.
2. Edit `~/dev/fabro-tofu/modules/fabro-app/templates/settings.toml.tpl`:
   `max_run_age_days` 7 -> 0.
3. `tofu plan` — review the shape (settings template + container
   recreate), then `tofu apply`.
4. Commit the variables pin same session.
5. Verify: the container boot line reads
   `run retention supervisor started max_run_age_days=0 enabled=false`.

The apply recreates the server container — wait for in-flight runs
(the develop lap on #1434 was in flight at session end) AND note the
recreate HEALS the tick wedge (work#1510): after the recreate, check
`fabro automations status` — the triager tick should attempt again.

After work#1514 deploys: restore `max_run_age_days = 7` (same
ceremony, back on 7 — archive-gated by then).

## Sprint 93 state (the ledger carries the record)

- Counted via #1482 (verify.nu ADDED-file classifier, PR #640 merged).
- **The sprint-93 ledger commit (2bb728e6d, iterate-state.json) is
  LOCAL on denkhaus, UNPUSHED** — the push gate refused (2 active
  runs at session end). First duty alongside the tofu stopper:
  re-check `nu .fabro/scripts/push-gate.nu` and push when open.
- The %3 architecture gate is DUE at 93 and DEFERRED with rationale
  (the session ended before the ceremony; the marker stays at 92).
  Open the next session with it, or defer again with rationale.
- Retro: WAIVED per user directive 2026-11-20 (retros never carry
  into a new session). Candidates for the NEXT sprint's retro:
  friction-score needs the tree CLI (the installed fork.42 binary
  lacks the new work-list flags — run it via
  `PATH="target/debug:$PATH" nu .fabro/scripts/friction-score.nu`),
  the validator-seam miss pattern (work#1478's own change missed the
  petri schema list), the mass-park disposition burden (seven manual
  rounds today — the all-PASS auto-close design change is worth a
  grill).

## Open threads

- #1509 (validator seam: terminal_cleanup missing from petri's
  WORKFLOW_SECTION_KEYS) + #1463 (cook wait function: the
  daemon-presence re-check): the Rust pair, lane:develop builds both.
  #1509 unblocks develop's cleanup declaration (the gear runs on the
  reverted toml since PR #639).
- #638 (PARKED, no label: the gear-fleet terminal_cleanup adoption +
  the /tmp sweep): lands AFTER work#1509 deploys — label
  merge:enabled then, and develop's declaration returns via that PR.
- #1510 (per-automation tick wedge): intermittent (the merger tick
  recovered by 13:40, the triager tick still silent as of 15:50).
  The recreate tonight heals it — verify and update the item.
- Watchdog breaker count 1 (park|workflow_error, the 11:00 run) —
  not yet diagnosed; the parked run needs a look or an archive.
- #1450 (closing-pass backoff, nu-only — session-side work) and the
  auto-close-when-all-PASS park design question: seven manual park
  dispositions today argued for it; grill before building.
- The develop lap on #1434 (soft closure-radius links) was IN FLIGHT
  at session end — its outcome rides the line.
