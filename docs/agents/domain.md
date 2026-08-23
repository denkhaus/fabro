# Domain Docs

How the engineering skills should consume this repo's domain documentation
when exploring the codebase. Layout: **multi-context**.

## Contexts

`CONTEXT-MAP.md` at the repo root (created lazily by /domain-modeling) points
to one CONTEXT.md per context. The contexts:

| Context | Where | What it owns |
| --- | --- | --- |
| Engine | `lib/` (Rust workspace: apps/components/foundation) | run, stage, checkpoint, sandbox, workflow engine vocabulary |
| Web | `apps/fabro-web`, `lib/packages/fabro-api-client` | SPA, API client, run/session UI vocabulary |
| Lab | `denkhaus-lab` branch + `docs/lab/` on `denkhaus` | workflow dogfooding; gofib product world |
| Marketing/docs | `apps/marketing`, `docs/public` | site and public docs vocabulary |

## The Lab contexts

- **Product world**: the `denkhaus-lab` branch — gofib CLI plus its
  `CONTEXT.md` (gofib, Fib, text/JSON mode, count flag, quality gate).
  Read with `git show origin/denkhaus-lab:CONTEXT.md`. Do not duplicate
  its terms into root docs.
- **Platform world (rescued)**: `docs/lab/` on `denkhaus` — the retired
  meta branch's durable home. ADRs 0001–0008 live in `docs/lab/adr/`
  (numbering continues there: ADR-0009+), the engine docs snapshot in
  `docs/lab/fabro/`, review reports in `docs/lab/reviews/`. New lab
  platform decisions go to `docs/lab/adr/`.

## Before exploring, read these

- `CONTEXT-MAP.md` at the repo root if it exists; then the per-context CONTEXT.md relevant to the topic.
- `docs/adr/` at the repo root (system-wide decisions) — created lazily; absence is fine.
- Lab work: `origin/denkhaus-lab:CONTEXT.md` and `docs/lab/adr/` in this tree.

If any of these files don't exist, proceed silently. Don't flag their absence;
don't suggest creating them upfront. `/domain-modeling` (reached via
`/grill-with-docs` and `/improve-codebase-architecture`) creates them lazily
when terms or decisions actually get resolved.

## Use the glossary's vocabulary

When your output names a domain concept (in an issue title, a refactor
proposal, a hypothesis, a test name), use the term as defined in the relevant
CONTEXT.md. Don't drift to synonyms the glossary explicitly avoids (the
_Avoid_ lines). If the concept you need isn't in the glossary yet, that's a
signal: either you're inventing language the project doesn't use (reconsider)
or there's a real gap (note it for /domain-modeling).

## Flag ADR conflicts

If your output contradicts an existing ADR, surface it explicitly rather than
silently overriding:

> _Contradicts ADR-0007 (event-sourced orders), but worth reopening because…_
