# Self-host Resource Planning (docker-compose / VPS)

Date: 2026-08-20
Status: measured on `ghcr.io/fabro-sh/fabro:nightly` (0.331.0-nightly.0) on a
compose deployment with the bundled `docker-compose.yaml`

This document records the resource footprint of a self-hosted Fabro deployment
so VPS sizing does not have to be guessed. The server itself is cheap; the run
containers dominate everything.

## Measured baseline (idle server)

| Component | Measured / verified |
|---|---|
| Server process (idle, after install wizard + doctor) | ~31 MiB RAM, ~5% of one core |
| Server image | 217 MB |
| `/storage` after init | ~256 KB |
| Steady-state network | KB/s (LLM API traffic) |

A small slice (0.5 vCPU, 512 MB) always covers the server process itself.

## Cost drivers: run containers

Defaults come from `DockerSandboxOptions::default()`
(`lib/components/fabro-sandbox/src/docker.rs`):

- Default run image is `buildpack-deps:noble`: ~0.27 GB compressed, roughly
  600 MB unpacked in the Docker cache. Every custom `[environments.<slug>]`
  image adds its own cache entry.
- **CPU and memory limits default to `None` (unlimited)** unless
  `[environments.<slug>.resources]` sets them.
- Scheduler default is `max_concurrent_runs = 8`
  (`[server.scheduler]`, server-owned settings).

Per-run workspace growth for a repo like this one (measured):

| Item | Size |
|---|---|
| Fresh clone (incl. `.git` ~51 MB) | ~170 MB |
| `bun install` (web deps) | ~750 MB |
| Rust `target/` after release builds | up to ~6.7 GB |
| Checkpoints (git-based, in `/storage`) | MB-scale per run, unbounded without prune |

Fabro's own dogfooding environment for this repo (the server-managed
`toolchain` environment resource) requests 8 CPU / 16 GB RAM - a useful
reference for the "agents compile Rust" case.

## Sizing profiles

| Profile | vCPU | RAM | Disk | Fits |
|---|---|---|---|---|
| Light | 2 | 4 GB | 40 GB SSD | agent text work, small repos, scripts |
| Medium | 4 | 8 GB | 80 GB SSD | 2-3 parallel runs, TS builds |
| Heavy (this repo) | 8 | 16 GB | 120 GB SSD | 1-2 parallel Rust builds |

RAM is the binding constraint, not CPU (runs mostly wait on LLM responses).
Disk should be an SSD (cargo builds on HDD are painful). Plan 2-4 GB swap as an
OOM buffer.

## Mandatory knobs on a small host

```toml
# server settings.toml - cap the default of 8 concurrent runs
[server.scheduler]
max_concurrent_runs = 2
```

```toml
# server-managed environment resource - run containers are unlimited by
# default; create via the environments API / `fabro ps` UI instead of
# project.toml ([environments.*] there is not transmitted to runs)
# PUT /api/v1/environments/docker-dev
{
  "id": "docker-dev",
  "provider": "docker",
  "resources": { "cpu": 2, "memory": "4GB" }
}
```

## Disk hygiene

- `fabro system df` - inspect server storage usage
- `fabro system prune --before YYYY-MM-DD` - delete old runs (checkpoints are
  git-based under `/storage` and grow without a prune policy)
- `docker image prune` - reclaim unused run images on the host daemon
