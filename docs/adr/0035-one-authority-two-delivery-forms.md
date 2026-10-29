# ADR-0035: One engine authority, two delivery forms — deterministic steps get the same channel as agent steps

- Status: Accepted (2026-10-26, user decision in the cutover design
  discussion; options A/B/C weighed, C chosen)
- Deciders: user (direction C, quoting the two-delivery-forms argument),
  agent (options paper: stdin packets vs scoped token vs channel shim)
- Relates to: ADR-0032 (central work tracker — the run tools whose
  authority this shares), ADR-0019 (capability gates — this is a
  capability delta WITH a recorded user decision), ADR-0017 (engine stays
  tool-agnostic — the channel is transport, not tool semantics)
- Evidence: slice 1 landed as denkhaus/fabro b0b95db2a —
  `RunChannel::spawn` binds the per-run Unix socket, the stamping
  layer replaces any client-supplied Authorization with the run's
  worker token (`run:worker` + `agent:run_tools`), `stop()` ends the
  accept loop and removes the socket; the channel IT pins the scope
  matrix and the stamping discipline over the sandbox wire
  (bearer-less UDS HTTP). The claim-binding core landed as
  denkhaus/work#991: the per-call policy below is now ENFORCED, not
  promised — `resolve_caller_project` binds the worker's requests
  (reads included) to the project its run's target repository names,
  close/update require a live claim held by the run, and grant/next
  pin the body runId to the authenticated run. Evidence: four
  module-level binding pins (each mutation-proven RED) plus wire-level
  ITs answering foreign project / foreign body runId / another run's
  claim with 403 and the scope in the message, never served. The
  SPAWN caller and the sandbox mount landed as denkhaus/work#989
  (the terminal drain landed as denkhaus/work#1003: run-keyed
  registry, idempotent take, drains at the three terminal sites):
  the run launch spawns the channel before any sandbox spec exists,
  the socket lives at `<channel-root>/<run>/channel.sock` (settings
  `[server.run_channel]`, env override `FABRO_RUN_CHANNEL_DIR`, kill
  switch `enabled = false`), and `RunChannelMount` applies it at
  sandbox create — Docker gains the per-run DIRECTORY bind at
  `/run/fabro` (a directory bind survives a crash-rest re-bind) plus
  `FABRO_SERVER=/run/fabro/channel.sock`, Host gains the env only,
  Daytona stays honestly absent, and NO bearer material ever enters
  the sandbox env (the only channel value that crosses to the sandbox
  is `FABRO_RUN_CHANNEL_SOCK`, and it carries the socket PATH, never a
  token; the run's worker token is minted server-side and stamped
  internally — no secret value crosses the process boundary). Evidence: five inline
  mount pins (Docker env+bind, binds append, Host, Daytona,
  no-bearer), the production-spawn IT (layout, health, kill switch,
  env override, awaited stale stop before respawn) and the CLI
  channel IT — the compiled `fabro` runs list/show/create/close over
  the socket with `FABRO_SERVER` as its ONLY credential and is
  refused a foreign project; all three mutation-proven RED. The
  review's duplicated
  "Transport protocol" line is resolved: one bullet below, matching
  the served implementation.

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

**Option C — a per-run capability channel into the sandbox: the
universal bridge between deterministic workflow steps and the engine,
with no secret material anywhere in the sandbox.**

- The engine exposes one per-run capability endpoint (a Unix socket
  mounted into the run's sandbox by the sandbox driver; engine-side
  lifecycle: created with the run, closed at terminal state).
- The `fabro` CLI ships in the toolchain image and detects the channel
  (e.g. `FABRO_RUN_CHANNEL`): CLI calls in a sandbox route over the
  channel instead of a bearer token. Same verbs, same UX as the host —
  different transport.
- **Parity principle — the channel is not a capability list.** The
  engine-side handler stamps the run's worker principal (`run:worker`
  + `agent:run_tools`) into every request internally; the existing
  principal middleware and route extractors ARE the authorization. The
  channel therefore exposes exactly what the run's agent could reach
  through its run tools — no more, no less. Every engine capability
  that becomes agent-reachable (work items today; whatever the engine
  grows tomorrow) arrives for deterministic steps in the same breath.
  Work items are the FIRST consumer (the cutover, ADR-0032); they
  define nothing about the channel's shape.
- Transport protocol: HTTP over the socket, served by the same axum
  router the TCP server serves (axum serves Unix listeners; the
  internal stamping layer is the production twin of the test
  router's user-bearer injection). The sandbox CLI needs a UDS
  transport in fabro-client; the wire is the API as-is.

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

- **The channel is the universal bridge, not a work-item service** (user
  amendment 2026-10-26): deterministic steps reach the ENGINE through
  it — work items are the first consumer, and every future
  agent-reachable capability flows to scripts automatically under the
  parity principle.
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

## Amendment (2026-10-29, denkhaus/work#1043): one handshake token in the owning run's env

The Decision's "no secret material anywhere in the sandbox" holds for
BEAR material and needs one named exception for transport: on the host
provider every run shares the filesystem and the server's uid, so the
connection alone is no authority — run A could open run B's socket and
be served under B's worker principal (the principal-confusion case the
parity principle assumes away by isolation).

Each spawn therefore mints a per-run HANDSHAKE token
(`FABRO_RUN_CHANNEL_TOKEN`, hex, transport credential only): the serve
refuses any client whose first message does not carry it on
`x-fabro-run-channel-token`, and the token enters ONLY the owning run's
environment — the worker's launch spec and the sandbox mount alike (a
remote sandbox that cannot reach the socket gains neither). The worker
BEARER keeps its original rule unchanged: minted server-side, stamped
server-side, never leaving the server process; client Authorization is
still replaced, never honored.
