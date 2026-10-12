# ADR-0028: Probe-only provider window gate

Date: 2026-10-12
Status: accepted (user decision, evening session)
Supersedes: the fabro-2f03 announce rule (2026-10-10, seed-recorded; never an ADR)

## Context

The line gate decides whether an automation fire may create a run when a
required LLM provider sits in a closed usage window. Two designs competed:

1. Periodic pair-probes (user decision 2026-09-14): every 10 minutes, one
   bounded one-word request per (provider, model-selector) — provider plus
   the stage's observed model since fabro-99a2 option A. Fire when open,
   hold when closed.
2. First-hand announce (fabro-2f03, 2026-10-10): a run whose conclusion
   announces a usage window is treated as evidence the window is spent,
   outranking probes.

Design 2 was added after one fire burned inside the sub-cadence gap (a
window closed between two probes). It then required four patches in two
days (99a2 selector fix, f73b naive-prose TTL, an ac3f generalization
fork), produced a 17-hour lane wedge (a timezone-naive reset prose can
never expire on its own), and its own failure modes burn more fires than
the one-per-closure-event they prevented.

## Decision

The gate is probe-only. The announce mechanism is removed from the fire
path (`manual_window_refusal`, `last_run_window_signal`, `window_signal`).
Periodic pair-probes are the single window authority.

Accepted residual: a fire admitted inside the sub-cadence gap after a
window closes can burn one run. It parks visibly (quota-park machinery)
and the next probe holds the lane until the window reopens. This cost is
accepted for every provider uniformly; no provider-specific constants,
no prose parsing, no timezone assumptions in the fire path.

Run-side window classification (parks, breaker exemption) is unaffected.

## Consequences

- Any future patch to window gating must cite this ADR; a second patch on
  the same mechanism triggers simplify-or-revert, not a third patch.
- Provider-specific window knowledge (lengths, timezones, prose formats)
  stays out of the gate. Providers that want exact reopen times may emit
  offset-carrying deadlines for information only.
