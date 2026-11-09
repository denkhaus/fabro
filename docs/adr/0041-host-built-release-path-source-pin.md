# ADR-0041: Host-built release path — the source pin replaces the digest pin

- Status: Accepted (2026-11-09, design session during the outage window;
  production down since 2026-11-08 22:50 by user decision — dark until
  the new path is built, no deadline, "everything is experimental")
- Deciders: user (product framing and all four shape decisions,
  2026-11-09), agent (facts, drafting)
- Relates to: ADR-0034 (vendored fork crates — unchanged by this ADR),
  ADR-0042 (run-built environment images — the toolchain image stops
  being a deploy artifact there), the 2026-09-12 grill decision that
  made the ghcr digest pin the reproducibility contract (revised here)

## Context

The release pipeline builds everything on the dev machine: the server
image via `cargo dev docker-build` (zigbuild inside containers, binaries
staged, `docker build` copies them — no source in the runtime image's
context), pushed to ghcr; the toolchain image cooked and pushed the same
way, 2.1 GB per release over the home uplink. fabro-tofu pins the fabro
image by ghcr digest and pulls it daemon-side through an SSH tunnel with
a ghcr-auth module (`docker login` + `docker pull` via remote-exec).

The 2026-11-08 deploy window broke on this design three ways at once: a
crane push produced a corrupt `fabro:b5a5d00a26dd` tag, the host's
`/root/.docker/config.json` was overwritten with the dev machine's
(credsStore pointing at a Windows helper), and the fallback transfer — a
nohup'd rsync of the 2.1 GB toolchain tar — died at 45% with its log
lost. The failed apply destroyed the fabro container; production went
dark.

Two facts make the dev-machine leg unnecessary: the compile pipeline is
fully containerized (`cargo dev docker-build` runs every step in
containers and works unchanged against any Docker daemon with disk), and
the host is a competent build runner (8× EPYC 9634, 262 GB free) with a
fast network. The fabro repository is public, so cloning it on the host
needs no credential.

## Decision

The release path moves to the host. The git sha becomes the
reproducibility anchor; the artifact digest stops being one.

1. **Host-autark release recipe** (fabro repo, beside
   `scripts/image-release.nu`): anonymous HTTPS clone of the public
   repository on the host at the pinned sha, the whole build inside
   containers against the host daemon (`cargo dev docker-build` shape),
   local tag `<version>-<sha12>`. No dev-machine orchestration, no
   deploy key, no registry in the critical path.
2. **Tofu consumes the local sha12 tag** instead of a ghcr digest pin:
   the docker image resource names the locally-built tag. Upgrades are
   deliberate sha bumps, same discipline as the digest bumps were.
3. **ghcr becomes an optional mirror, default on**: after a successful
   deploy the host pushes the server tag with `docker push` (daemon
   push, not crane). Disaster rollback stays a pull; mirroring can be
   switched off per deploy.
4. **Rehearsal before first live apply**: production being dark leaves
   the host daemon free — the recipe is exercised end-to-end (build,
   tag, dry apply) before the apply that restores production.

## Alternatives rejected

- **Dev-machine orchestration against the host daemon**
  (`DOCKER_HOST=ssh://…`): zero tofu change, but the dev machine stays
  the pacemaker, build contexts travel per build over SSH, and the
  local-build pain the user named survives.
- **Tofu builds inside the apply** (`null_resource` + remote-exec):
  the apply would hold a 30–60 min SSH provisioner open with state
  churn on abort — the same fragility class that blew the window.
- **Keep the digest pin and ghcr in the path**: the corrupt-crane and
  pull-auth failure classes stay load-bearing for every deploy.

## Consequences

- Deploys no longer depend on ghcr pull auth; the ghcr-auth tofu module
  retires from the critical path (it stays for the optional mirror
  push's credentials).
- Host loss means a fresh host plus a recipe rerun (~1 h), not a
  registry pull — the source repository is the source of truth.
- Builds are not bit-identical; identity rides the sha12 tag name
  (which source sha the image was built from). Reproducibility is by
  source pin plus locked manifest, not by artifact digest.
- `image-sweep` housekeeping is unchanged (keep current + previous).
- fabro-tofu stays a separate repository: moving it in would require
  making the fabro repository private first (deployment details stay
  private — user, parked mittelfristig 2026-11-09).
