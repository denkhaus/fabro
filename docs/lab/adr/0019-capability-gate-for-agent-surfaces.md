# ADR-0019: Capability gate for agent surfaces

- Status: Accepted
- Date: 2026-09-08
- Deciders: user + agent (fabro session)
- Related: ADR-0018 (seed ownership), ADR-0017 (tool-agnostic engine), fabro-06e0 (gh removal), PR #53 (the incident)

## Context

On 2026-09-07 the revisor proposed, and the develop line merged (PR #53),
baking the `gh` CLI into the agent toolchain image — chosen by the
implementer from two seed options, without a user decision, and without
anyone challenging the capability delta. A follow-up seed even proposed
provisioning `GH_TOKEN` into run sandboxes. The user rejected this
direction: agents must not receive tools with which they could cause
damage. The governance gap: ADR-0018 regulates WHO works on a seed, but
nothing regulated WHICH CAPABILITIES the work may hand to agents. The
line could arm its own sandboxes.

## Decision

1. **Least privilege is the default.** Agent sandboxes (toolchain image,
   environment env, workflow tool allowlists) carry only what the concrete
   task minimally requires. No "nice to have" tools, no speculative
   credentials.
2. **Write capabilities are engine-mediated only.** Every GitHub write
   operation flows through `fabro_github` on the server (run PR creation,
   auto-merge, gate enforcement, branch updates, PR-state polling for
   `fabro_run_wait`). Agents get NO raw authenticated clients — no `gh`
   with token, no token-bearing curl, no API keys in agent shells.
   Read-only exceptions require explicit user approval recorded on the
   seed.
3. **Direct agent tools are read-only at most** (user sharpening,
   2026-09-08): an agent-facing tool may expose derived READ data through
   the engine (e.g. run lists, PR state via `fabro_runs_list` /
   purpose-built read-only tools), never credentials or general clients.
   Every forge WRITE stays engine-side — the agent expresses intent
   (create run, wait for merge), the engine executes under its own
   governance.
4. **Capability additions are user decisions.** Any seed or PR that adds,
   removes, or changes a tool, credential, or permission in an
   agent-reachable surface (Dockerfiles, environment env, tool
   allowlists, hook configs) is labelled `needs-user` and stays
   unassigned until the user approves it (ADR-0018 D3 ceremony).
5. **Reviewers watch for capability deltas.** The develop reviewer and
   the line-watch monitor treat tool/credential changes in agent surfaces
   as review findings that block approval until the user decision is
   recorded.

## Consequences

- `gh` is removed from the toolchain image (fabro-06e0); the in-flight PR
  guard uses the engine-mediated, read-only `fabro_runs_list` instead.
- The engine's existing mediation (fabro-github crate: server-side only)
  is the reference pattern: agents receive purpose-built mediated tools
  (fabro_run_create/wait/get/gather), never general clients.
- The assignee filter (ADR-0018) gains a second axis: capability seeds
  are unassigned + needs-user even when their category would otherwise
  be line work.
- Violations are process defects: a merged capability change without a
  recorded user decision gets reverted, not ratified.
