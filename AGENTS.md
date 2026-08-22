# AGENTS.md

This file provides guidance to AI agents working in this repository.

The `denkhaus-lab` branch is a minimal workflow playground: `.fabro/` workflows,
project config, and tooling setup only. There is no application source code here.

## Agent skills

### Issue tracker

Issues are tracked with seeds (`sd` CLI, `.seeds/issues.jsonl`). See `docs/agents/issue-tracker.md`.

### Triage labels

Default Matt Pocock label vocabulary, identity-mapped. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one `CONTEXT.md` + `docs/adr/` at the repo root. See `docs/agents/domain.md`.
