# ADR-0017: The engine stays tool-agnostic about project-scope tooling

- Status: Accepted
- Date: 2026-09-07
- Deciders: user + agent (fabro session)
- Related: fabro-66db, ADR-0012 (dogfooding), ADR-0013 (upstream posture)

## Context

The fabro-66db painpoint: run-container tool shims refused every command
until someone ran `mise trust`, and the planner burned LLM turns
discovering that bootstrap. The first fix attempt ran `mise trust`
directly inside the sandbox providers (`docker.rs`, `daytona/mod.rs`) —
a layering violation. mise exists only in this repository's project
scope (`.mise.toml`, `.fabro/Dockerfile.toolchain`); the sandbox layer
is generic engine code serving any GitHub project. The user rejected
the engine-side fix (2026-09-07): "mise gibt es nur im Projekt scope
und spielt keine Rolle für fabro an sich — fabro muss tool-agnostisch
bleiben."

## Decision

1. Engine components (sandbox providers, workflow engine, server, CLI)
   never reference project-scope tools by name or behavior — no mise,
   asdf, direnv, nvm, or equivalent ever appears in engine seams.
2. The engine's stable, documented contract for bootstrap purposes is
   the clone-based workspace layout: repository at
   `/repos/<owner>/<repo>`, execution symlink `/workspace/<repo>`.
   Project artifacts may rely on this layout for configuration.
3. Project tooling bootstrap lives in project artifacts: the toolchain
   image (`.fabro/Dockerfile.toolchain`), server-side environment
   `env` config, or workflow hooks/scripts in `.fabro/`.
4. Concrete case (fabro-66db): `.fabro/Dockerfile.toolchain` sets
   `ENV MISE_TRUSTED_CONFIG_PATHS=/repos/denkhaus/fabro`. Path-based
   trust is content-independent (a baked `mise trust` hash would go
   stale on every `.mise.toml` edit). Verified in pristine containers
   (mise 2026.8.10): this setting accepts exactly one path or
   directory — no globs, no comma-separated lists — so the image pins
   this repository's canonical clone path; the image is repo-scoped
   anyway (it bakes this repo's toolset pins).

## Consequences

- Any future "run tool X bootstrap" demand maps to project artifacts
  first, never to engine seams.
- A generic engine mechanism (for example an environment-configured
  post-clone command) is the only acceptable engine-side evolution, and
  only when image/env cannot express the need.
- The pinned path relies on the documented clone layout; changing the
  `/repos` layout or the repository identity is a contract break that
  must revise this ADR and the toolchain image.
