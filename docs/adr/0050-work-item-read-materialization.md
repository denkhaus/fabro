# ADR-0050: Work-item read materialization (amends ADR-0032)

## Status

Accepted (2026-11-21, grilled in session with the operator; Jev
pre-screen advisory, user-confirmed on all three forks).

## Context

ADR-0032 decided the tracker provider owns work-item content and the
server owns only the claim ledger ("content is owned by a tracker
provider, never by the server"). Reads pass through to the forge
one-to-one. The premise was that pass-through always works.

On 2026-11-21 the premise failed: the five-gear fleet plus the
session exhausted the GitHub App installation token's hourly window
three times; runs died at READS — most tellingly the tracker-resilience
lap itself passed its quality gate and then died at the evidence
capture's `work show` (403 rate limit exhausted), stranding a
green-gated +636-line implementation.

## Decision

Fabro materializes a READ VIEW of work-item content in its own
database. Reads resolve cache-first; the forge is loaded on demand.

1. **Write-invalidation only, no TTL** (operator choice): a cached
   item is fresh until one of our own writes moves it (write-through:
   the forge write and the cache update are one action) or an explicit
   refresh is requested. Foreign edits made directly at the forge
   stay invisible until an explicit refresh — accepted deliberately
   for maximum quota savings; the operator works the tracker through
   fabro.
2. **First cut: item content only** (title, body, labels, state) —
   the read class that kills runs today. Comments and links are part
   two.
3. **Ownership unchanged**: writes still go to the tracker, which
   stays the source of truth. The claims-ledger principle of
   ADR-0032 widens from "claims only" to "claims + read view".
4. **Degradation**: when an on-demand load fails (quota, outage), the
   cached content serves with an honest staleness indication, never a
   hard failure. The typed tracker error classification (work#1556)
   sharpens the message later; the serve-stale behavior itself is
   failure-class independent.

## Consequences

- The evidence capture, planner packets, queue views, and the
  acceptance fold (work#1555) read locally; a quota-exhausted forge
  changes nothing for readers.
- The materialized rows live beside the claim ledger in the fabro DB;
  the cache is a projection of tracker content, authoritative for
  nothing the tracker can still answer.
- An explicit refresh door exists for the foreign-edit case.
- ADR-0032's letter is amended by this ADR where it said "never by
  the server": content ownership stays with the tracker; the server
  holds a materialized read view.
