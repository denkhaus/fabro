# Docker Provider: Auto-Build Runner Images from `image.dockerfile`

Seed: `fabro-969f` · Plan: `pl-09be` (approved) · Created: 2026-08-23

Child seeds:

1. `fabro-72a0` — Config plumbing: map `image.dockerfile` into `DockerSandboxOptions` and validate provider rules
2. `fabro-3822` — Core: `ensure_image` builds `fabro-runner-<sha12>` on first use and reuses on content-hash hit
3. `fabro-829a` — Docs and lab validation: `environments.mdx`, `run-configuration.mdx`, mise runner example

## Summary

`DockerfileSource` (`image.dockerfile`, inline or path) is consumed only by the Daytona
provider (snapshot build + `dockerfile_sha256` cache key in `daytona::snapshot_identity`).
The Docker provider accepts only `image.docker` — a finished image — and ignores
`image.dockerfile` with a warning (see the provider matrix in
`docs/public/execution/environments.mdx`).

Repos that run docker sandboxes must therefore bake runner images outside Fabro
(manual `docker build` or external CI), or pay per-run toolchain setup. In the denkhaus
lab, `mise install` costs ~24.5s of every ~32s run setup (run `01M0NJZGX7BMJAJC3CZJZ0RNT0`,
events seq 13–14). Runner-image-as-config should travel with the repo for the docker
provider exactly like it already does for Daytona.

## Approach

Mirror the Daytona caching discipline inside the docker sandbox layer:

- `fabro-types` `EnvironmentImageSettings.dockerfile` → `run_compiler` already resolves
  `Path` references to `Inline` content from bundled workflow files (`resolve_dockerfile`),
  implemented for Daytona and reusable as-is.
- `fabro-sandbox::from_environment::docker_config_from_environment_env` maps the inline
  dockerfile into `DockerSandboxOptions` (new field, inline-only, like
  `DaytonaSnapshotSettings.dockerfile`).
- `DockerSandbox::ensure_image` gains a build branch:
  - Derive the tag `fabro-runner-<sha12>` from the inline content (same sha256 identity
    idea as `daytona::snapshot_identity`, but without API key / tenant scope).
  - `inspect_image` first: on hit, reuse without rebuild; on miss, build via bollard
    `build_image` with a minimal tar context containing only the Dockerfile.
  - Label the image with the Fabro managed label (`sh.fabro.managed`).
  - Emit the existing `SnapshotCreating` / `SnapshotReady` / `SnapshotFailed` sandbox
    events; no new event variants are needed (snapshot == image source).
  - Surface build stream errors through `crate::Error` (bollard reports build failures
    inside the `BuildInfo` stream, not always as transport errors).
- Validation in `fabro-config` `resolve/environment.rs`: a docker environment setting
  both `image.docker` and `image.dockerfile` becomes a resolve error (ambiguous image
  source), symmetric to the existing `daytona + image.docker` error. A `Path` dockerfile
  reaching the sandbox layer is an error, matching the Daytona invariant.
- Both entry paths — `fabro-workflow` `operations/start.rs` via
  `docker_config_from_environment_with_secrets` and `fabro-server` `run_manifest.rs` via
  `docker_config_from_environment` — flow through the same mapping, so one change covers
  CLI and server workers.

## Scope boundaries

In scope:

- inline-only dockerfile support for `provider = "docker"` (inline string and
  `{ path = ... }` resolved from bundled workflow files)
- content-hash tag derivation and reuse (`fabro-runner-<sha12>`)
- resolve-time error for `image.docker` + `image.dockerfile` on docker environments
- build failure surfacing as sandbox/preflight errors including build output
- documentation updates (`environments.mdx`, `run-configuration.mdx`)
- lab validation with the denkhaus mise runner

Out of scope:

- prune/GC of labeled `fabro-runner-*` images (`system prune` does not touch images
  today; file a follow-up seed)
- full build-context support (COPY/ADD of repo files)
- changes to the Daytona provider or its snapshot identity scheme

## Alternatives considered

- **Build during server preflight instead of sandbox initialize.** Rejected: preflight
  runs without a vault and duplicates daemon access; `ensure_image` in `initialize()` is
  the single choke point both the CLI run path and server workers already use, and it
  already implements pull-if-missing semantics the build branch can mirror.
- **Persist the built image reference in the environment store.** Rejected: the
  content-hash tag is self-describing and stateless, exactly like the Daytona snapshot
  name derivation; no new stored state to migrate or invalidate.
- **Keep `image.docker` + documented manual build script.** Rejected: that is the status
  quo the seed removes; runner-image-as-config should travel with the repo.
- **Full build-context support (COPY bundled files).** Rejected for this change:
  inline/path dockerfiles build with an empty context, the same constraint Daytona
  snapshots have; a context-bearing form needs manifest archive semantics and is a
  separate feature.

## Risks

- bollard `build_image` reports failures inside the `BuildInfo` stream (`error` field),
  not always as transport errors. Fold stream error text into the `crate::Error` context,
  mirroring how `create_image` pull errors are wrapped today.
- Inline-only, empty build context means COPY/ADD cannot reference repo files. Document
  the limitation and the error path rather than inventing context semantics.
- A `Path` dockerfile reaching the sandbox layer un-inlined (future callers bypassing
  `run_compiler`) must fail loudly like `daytona::snapshot_identity` does.
- First-use builds can take minutes on a cold daemon and block sandbox initialize;
  `SnapshotCreating`/`SnapshotReady` events keep progress visible. Run setup duration
  metrics will shift — acceptable, that is the trade the feature makes.
- Content-hash-only tags are shared across projects on the same daemon: identical
  dockerfiles intentionally share one image; divergent dockerfiles accumulate images.
  Labeling enables later GC; pruning stays out of scope.

## Acceptance criteria

- `image.dockerfile` works for `provider = "docker"` in both forms: inline string and
  `{ path = ... }` resolved from bundled workflow files.
- The image is built once per content hash: tag is `fabro-runner-<sha12>` of the
  dockerfile content, and an unchanged dockerfile produces no rebuild on subsequent runs
  (verified via sandbox events / `inspect_image` reuse).
- Setting both `image.docker` and `image.dockerfile` on a docker environment fails config
  resolution with a clear error; setting neither keeps today's default image.
- Build failures surface as clear sandbox/preflight errors including the docker build
  error output.
- `docs/public/execution/environments.mdx` and `run-configuration.mdx` document the
  field, the caching behavior, the empty build context, and the both-set rule.
- A lab-validated example (denkhaus lab mise runner) with its run id is linked in seed
  `fabro-969f`.
- `cargo nextest run -p fabro-sandbox -p fabro-config` passes; a real-daemon build/reuse
  test is included behind the existing `#[ignore]` convention.
