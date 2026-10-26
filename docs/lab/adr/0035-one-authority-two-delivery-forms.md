# ADR-0035: One engine authority, two delivery forms — deterministic steps get the same channel as agent steps

- Status: Accepted (2026-10-26, user decision in the cutover design
  discussion; options A/B/C weighed, C chosen)
- Deciders: user (direction C, quoting the two-delivery-forms argument),
  agent (options paper: stdin packets vs scoped token vs channel shim)
- Relates to: ADR-0032 (central work tracker — the run tools whose
  authority this shares), ADR-0019 (capability gates — this is a
  capability delta WITH a recorded user decision), ADR-0017 (engine stays
  tool-agnostic — the channel is transport, not tool semantics)
- Evidence: to come with the implementation (the capability-channel unit,
  denkhaus/work items); this ADR records the DIRECTION decision.

## Context

Stage execution has two shapes with different requirements:

1. **LLM steps** call engine capabilities through the run tools
   (`fabro_work_next/show/create/update/close/release`, …): the agent's
   tool loop speaks the tool protocol; the ENGINE executes each call
   with the run's authority and enforces the run's boundary
   server-side (claim-bound closes, project-bound creates).
2. **Deterministic steps** (nu scripts in the sandbox: closeout,
   evidence, planner-preflight, loop gates) need the SAME capabilities
   from a process, not an LLM loop.

The seeds era solved (2) by shipping the `seeds` binary plus the shared
file store into the toolchain image — a local store needs no channel.
The tracker cutover (ADR-0032) removed the store; the interim answer
was **stdin packets**: the agent calls a tool and forwards the result
to the script. That answer is structurally weak: fail-closed gates
verify agent-forwarded COPIES, not server truth (a prompt-injected
agent can malform a packet); filings and closes depend on agent
compliance between script and tool call; and every prompt carries
fragile forwarding choreography (the fabro-c42f packet-drop class).

## Decision

**Option C — a per-run capability channel into the sandbox, no secret
material anywhere in the sandbox.**

- The engine exposes one per-run capability endpoint (a Unix socket
  mounted into the run's sandbox by the sandbox driver; engine-side
  lifecycle: created with the run, closed at terminal state).
- The `fabro` CLI ships in the toolchain image and detects the channel
  (e.g. `FABRO_RUN_CHANNEL`): `fabro work …` in a sandbox routes over
  the channel instead of a bearer token. Same verbs, same UX as the
  host — different transport.
- The engine-side handler is THE SAME authorization core the run tools
  use: one policy (run-scoped, claim-bound, project-bound), two
  presentations (tool protocol for LLM steps, channel RPC for
  deterministic steps).
- Transport protocol: thin JSON-RPC-shaped request/response (final
  shape is the implementation unit's to settle; MCP reuse was weighed
  and left open — scripts need a handful of verbs, not a tool
  negotiation).

## Options weighed

- **A — stdin packets (interim status quo):** zero new capability, but
  an integrity hole (gates trust agent copies) and permanent LLM
  dependence in prompts. Rejected as the end state; stays until C
  lands.
- **B — run-scoped fabro API token in the sandbox env:** strong
  integrity, precedent exists (the engine already injects a scoped
  `GITHUB_TOKEN` per run — `fabro-github/src/access.rs`). Rejected for
  now: an exfiltratable bearer credential in every sandbox is a bigger
  capability class than a positional channel whose authority dies with
  the run; B remains the documented fallback if C's channel plumbing
  proves disproportionately expensive.
- **C — chosen:** authority stays engine-side, no secret to leak
  (nothing to copy out; the channel exists only inside this sandbox),
  per-call authorization identical to the run tools, trivially
  revoked (socket closes). Cost: new sandbox-driver plumbing (socket
  facet, lifecycle) — the largest engineering item, paid once, and it
  makes every future engine capability available to deterministic
  steps for free.

## Security posture (ADR-0019 axis)

This IS a capability delta — recorded here as the user decision the
reviewer axis demands. Reachability inside the sandbox is equivalent
to B's (any process in the container can talk to the socket); the
difference is exfiltration (nothing to carry elsewhere), attribution
(every call is the run's), and revocation (run-terminal). The
per-call policy — NOT token claims — is the enforcement point: read
yes, create in the run's project yes, close/release ONLY the run's own
claim, run lifecycle NO, foreign projects/claims NO.

## Consequences

- The workflow-asset rewrite (develop/loop/revisor onto work items)
  targets the CHANNEL, not packets: closeout verifies the claim
  server-side, files residuals atomically, closes atomically — the
  fail-closed gates get STRONGER than in the seeds era. It waits for
  the channel unit.
- The toolchain image gains the `fabro` CLI (stage 7's rebuild is the
  natural vehicle; the channel ships with it).
- The stdin-packet paths in planner-preflight/evidence are interim and
  get replaced.
- Host scripts keep the bearer-token CLI (the host is a trusted
  actor); only sandbox transport changes.
