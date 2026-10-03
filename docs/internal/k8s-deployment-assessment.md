# Kubernetes Deployment Assessment

Date: 2026-08-20
Status: findings from a concrete evaluation (RKE2/containerd cluster, `ghcr.io/fabro-sh/fabro:nightly`)

This document records what it takes to run the Fabro server on Kubernetes today,
so the analysis does not have to be repeated from scratch. It reflects the
implementation as of `0.331.0-nightly.0`.

## Server pod: easy, PaaS-ready by design

The server component maps onto Kubernetes with minimal effort:

- Image is a static musl binary, runs as unprivileged user 1000, `tini` as PID 1,
  honors `$PORT` (explicitly built for PaaS; see `Dockerfile`).
- All durable state lives in `/storage` (SlateDB embedded, object store, dev
  token, JWT keys). No external database is required. One RWO PVC suffices.
- `/health` is unauthenticated and suitable for liveness/readiness probes.
- Documented shape in `docs/public/administration/self-host-docker.mdx`:
  one-replica StatefulSet (not Deployment) + PVC + Service + Ingress.

Constraints to respect:

- **Single replica only.** The server assumes exclusive ownership of `/storage`;
  there is no leader election. Use `strategy: Recreate` to avoid two writers
  during rollout.
- Losing the PVC loses the JWT signing keys, invalidating all tokens. Plan
  backups around the volume, not just config.
- `FABRO_WEB_URL` / `[server.web].url` must be set to the external origin for
  install links, browser auth, and webhook-triggered automations.

## Sandbox provider: the hard part

The Docker sandbox provider is clone-based and spawns **sibling run containers**
through the host daemon. Two implementation facts drive every option:

1. `DockerSandbox` connects via `Docker::connect_with_local_defaults()`
   (`lib/components/fabro-sandbox/src/docker.rs`), which on Unix resolves to
   `connect_with_unix_defaults()`. Bollard honors `DOCKER_HOST` **only when it
   starts with `unix://`** — a `tcp://` `DOCKER_HOST` is silently ignored and the
   client falls back to `/var/run/docker.sock`. There is no plain-TCP daemon path.
2. Run containers are created without published ports and addressed over the
   daemon API (exec/attach), so they only need to be reachable from the server
   process, not from outside the cluster.

RKE2/containerd nodes have no `docker.sock`, so the provider needs one of:

| Option | Effort | Notes / risks |
|---|---|---|
| DinD sidecar (privileged) | +3-6h | `DOCKER_HOST=unix:///var/run/docker.sock` via shared `emptyDir`. `docker/entrypoint.sh` already handles socket GID mismatches. Privileged pod must pass PSA/SCC policy; run containers die with the pod; image cache needs a PVC or it is `emptyDir`-ephemeral. |
| Remote daemon via socket bridge (socat sidecar -> dedicated Docker VM) | +2-4h | Cleanest lifecycle separation; daemon survives pod restarts. Never expose raw 2375 over the network - use SSH tunnel or TLS. |
| Daytona provider instead of Docker | ~0h | Server pod stays easy; sandboxes move to the cloud service. Removes all cluster coupling but adds an external dependency (and its credentials). |
| Local provider | n/a | No isolation and the Alpine server image lacks a toolchain; not a real option. |

DinD's internal bridge (172.17/16) does not collide with typical RKE2 pod CIDRs
(10.42/16).

## Bottom line

| Scenario | Effort | Risk |
|---|---|---|
| Server-only (Daytona sandboxes) | light, ~half a day | low |
| Server + DinD sidecar | medium, ~1-1.5 days | privileged pod; run-container lifetime == pod lifetime |
| Server + remote daemon | medium, ~1 day | daemon network security |

The server was built for container platforms; nearly all difficulty concentrates
in answering **"where do the run containers live?"**. For single-admin setups,
docker-compose on a VPS is the simpler deployment (see
`self-host-resource-planning.md`).
