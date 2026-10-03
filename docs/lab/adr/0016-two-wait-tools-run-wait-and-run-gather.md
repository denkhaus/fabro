# ADR-0016: Two wait capabilities — `fabro_run_wait` and `fabro_run_gather`

- Status: Accepted
- Date: 2026-09-06
- Deciders: user + agent (fabro session)
- Related: fabro-571e, ADR-0009 (per-node tool envelope), ADR-0013 (upstream posture)

## Context

Autonomous orchestration (conductor, ADR-0015) burned agent turns on
sleep/poll wait loops: `fabro_run_get` + shell `sleep` every 60–90 s for
child runs, and `git fetch` + tree-diff polling for pull-request
integration. The first autonomous merge pass (01M1SQYDJMHS) spent
~$0.30–0.50 per pass purely waiting.

The approved fix (fabro-571e, user decision 2026-09-05 option b) is a
blocking wait primitive. Mid-implementation we found the codebase already
ships a wait tool: `fabro_run_gather` (multi-run fan-in, terminal-only,
tool-side polling, ≤600 s) — part of the upstream feature set we maintain
and build on. Two overlapping wait capabilities needed an explicit
division of labor.

## Decision

1. Keep BOTH tools. Retiring `fabro_run_gather` was rejected: it is an
   upstream feature, public MCP surface, and documented API; we carry
   upstream features forward (ADR-0013 posture).
2. `fabro_run_wait` (new, fabro-571e): single run, `until=terminal` or
   `until=merged` (server-side PR state), server-side long-poll
   (`GET /runs/{id}/wait`), one call up to 3 600 000 ms, structured
   `reached` outcome (`terminal | merged | closed_unmerged | timeout`).
   The conductor legs wait on it: create -> wait -> route.
3. `fabro_run_gather` (unchanged): many runs, fan-in, terminal-only,
   short bounded waits (≤600 s). The revisor/select fan-in pattern keeps
   it.
4. Cross-references live in the tool descriptions (`fabro-tool`
   `common.rs`), the public docs (`child-runs.mdx`, `mcp.mdx`), and this
   ADR. Agents route by the shape of the wait, not by name affinity.
5. The server endpoint is the single wait mechanism for terminal state
   with bounded latency; a client-side request timeout must not preempt
   the wait deadline (`send_api_long_poll` in `fabro-client`).

## Consequences

- Two wait tools with disjoint specialties; descriptions and docs must
  keep pointing at each other when either changes.
- `until=merged` detection is server-side: the server tracks the run's
  PR link and polls GitHub; a PR closed without merging returns
  `closed_unmerged` so callers route to repair instead of hanging
  (fabro-94e8 owns automatic repair).
- If a future upstream release grows its own single-run wait, revisit
  this ADR before adding a third tool.
