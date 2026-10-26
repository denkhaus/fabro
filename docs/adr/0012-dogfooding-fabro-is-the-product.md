# Dogfooding: fabro is the product

Decided in the 2026-08-27 grill-with-docs session.

## Context

The lab product world has been a demo CLI (gofib on `denkhaus-lab`),
built seed-by-seed by the develop workflow to exercise the platform.
Upstream develops fabro within fabro itself (`fabro-sh/fabro`); those
dogfooding workflows are presumably upstream-only and private. We keep
developing our fork with our requirements in the meantime.

## Decisions

- **Direction: converge on dogfooding.** The product the develop
  workflow builds becomes fabro itself — product seeds change fabro
  code and workflow assets in this repository. We approach the target
  gradually ("tasten uns ans Ziel heran"); the gofib demo retires when
  the fabro-in-fabro loop is proven on our stack.
- **One product tracker.** The earlier lab/platform seed split for
  revisor output is superseded: with fabro as the product, revisor and
  improve findings file into the fabro repo's tracker. The platform
  tracker on `denkhaus` remains for engine work done in iterate cycles
  while the transition runs.
- **The d810 special case is closed.** "Not implementable from
  the product sandbox" was an artifact of the demo product; with fabro
  as the product, workflow-asset and code changes are equally
  implementable from the sandbox clone (upstream proves it).

## Consequences

- The quality gate in sandboxes must carry the Rust toolchain
  (upstream's setup is the reference; build-context seed fabro-199a is
  adjacent).
- Upstream posture is unchanged: we offer nothing until upstream reacts
  to our open PRs; dogfooding is ours, not an offer.
- The workflow-revisor (ADR-0011) targets develop runs on this repo and
  becomes the meta level of the same dogfooding loop.
