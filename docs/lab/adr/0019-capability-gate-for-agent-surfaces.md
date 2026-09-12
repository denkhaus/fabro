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
   the engine (run lists, PR state via `fabro_runs_list`), never
   credentials or general clients. Extend EXISTING tools rather than
   adding new ones (user decision 2026-09-08); a new agent tool is
   itself a capability addition requiring user approval.
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

6. **The sandbox stays credential-free** (user decision 2026-09-08,
   escalation): agent-authored configuration must not be able to GRANT
   credentials. The line's agents write their own workflow configs
   (`.fabro/workflows/**`) via PRs; a `[run.integrations.github]`
   permissions block in agent-authored config is self-escalation -
   especially combined with exfiltration channels (curl, web_fetch).
   The engine therefore honors token-minting ONLY for runs created by
   a User principal; Worker/System/Webhook/Slack-originated runs get
   the no-token path regardless of what their workflow config declares
   (enforcement seed filed). The sandbox offers access to repo contents
   only; any credential inside it (including clone-URL embedded tokens)
   is a defect to be engineered away.

7. **Engine-provided credentials and bridges are agent-reachable
   capability** (PR #68, run 01M20RK7XFNPJRZ2T437XGDEP8; fabro-16ff
   user directive, 2026-09-08): the `GITHUB_TOKEN` injected into run
   environments via `resolve_workflow_env`
   (`lib/components/fabro-workflow/src/services.rs`) and the git
   credential bridge (`lib/components/fabro-workflow/src/git_bridge.rs`)
   ARE capability surfaces within the meaning of this ADR. A diff that
   merely USES an engine-provided credential or bridge on a new code
   path is a capability delta requiring recorded user approval — it does
   not matter that the credential already existed or that the engine,
   not the agent, minted it.

8. **The post-#849 ambient checkout credential is accepted as the
   sandbox's forge capability** (user decision, 2026-09-12, fabro-0f22
   resolved): upstream's sandbox-driver adoption installs the clone
   token as the checkout's ambient credential (per-repo git-credential
   store, umask 077, remote URL stays clean; fabro agents can push from
   their own shells). The token is minted per repository with
   `contents: write` ONLY — it cannot open or merge pull requests
   (that requires `pull_requests: write`, which the engine mints as a
   separate server-side token that never reaches a sandbox). Blast
   radius of exfiltration: commit/branch writes to exactly the run's
   repository for the token's GitHub-side lifetime (installation tokens
   are fixed at 60 minutes; the engine refreshes at a 10-minute
   margin). No token revocation on sandbox teardown (explicitly
   declined); no per-agent credential scoping beyond the per-checkout
   store. The strict credential-free-sandbox posture of decision 6 is
   superseded for clone/push capability by this item; item 7's
   capability-gate discipline is unchanged.

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
- Prompts are agent-reachable surface and get rewritten by revision
  passes; this ADR — not the workflow prompts that restate item 7 — is
  the authoritative record of the engine-provided-credentials doctrine.
