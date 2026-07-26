---
id: dec-redact-at-source
status: proposed
owners: [bryan]
paths:
  - lib/crates/fabro-redact
  - lib/crates/fabro-workflow/src/event
  - lib/crates/fabro-server/src/server/handler/events.rs
---

# Redact at the source

Secrets are made safe where they are known, enforced once where data enters
shared storage, and trusted everywhere else.

## Context

Registry-based redaction (exact replacement of known secret values) is only
possible in the process that loaded the run's secrets — the worker, while
the run executes. It cannot be applied ex-post facto. Pattern-based
redaction (entropy + gitleaks) can run anywhere but is heuristic. As a
multi-tenant SaaS, the server never holds tenant secrets in plaintext, and
unredacted data at rest is an incident.

## Decision

1. **Redact at the source.** The process that knows a secret makes every
   emission safe before it crosses a boundary — wire, disk, or log. The
   worker applies registry + pattern redaction to events and command
   output. Errors never carry raw secrets or raw process output in their
   displayable text (raw material lives behind redacting accessors, e.g.
   `ExecOutputTail`); credential-bearing URLs travel as `DisplaySafeUrl`.
   Log sites format these already-safe values.
2. **Enforce once at ingest.** The server applies pattern-based redaction —
   the only kind it can perform — when events enter shared storage.
   Everything at rest is redacted.
3. **Trust everywhere else.** No redaction on read paths or log output.
   One backstop: a registry scrub on the worker's tracing subscriber,
   because third-party crates also log, and worker logs persist
   server-side.

## Alternatives

- Redact at read boundaries — rejected: no registry there, raw data at
  rest is unacceptable, and the mixed model already left the list-events
  endpoints returning raw payloads.
- Blanket pattern scrub of all log lines — rejected: heuristic cost for
  false confidence.

## Consequences

- Read-side re-redaction (SSE, event detail, CLI) is deleted, along with
  the `redacted` flag on event detail responses.
- `SecretRedactor` is populated from the worker's vault at run start.
- A value that slips both source and ingest passes is persisted
  permanently; pattern rules improve go-forward only. Inherent to the
  server never knowing tenant secrets — accepted.

## Guidance

New output channels (event types, logs, artifacts, API responses) redact in
the producing process. Never add read-time redaction to compensate for a
producer that doesn't.
