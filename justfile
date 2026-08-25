# Local build + deploy pipeline for the fabro server.
#
# `just up` does the whole loop:
#   1. refresh the embedded SPA (host bun, cached node_modules)
#   2. build the release CLI binary inside docker (cargo-zigbuild,
#      musl static — same binary as the release image)
#   3. build the local docker image
#   4. install the fresh binary as the user CLI at ~/.fabro/bin/fabro
#   5. docker compose up -d and wait for /health
#
# Caching (the slow parts survive repeated runs):
#   - cargo registry + build cache: docker volumes
#     fabro-docker-cargo-registry / fabro-docker-cargo-target-<arch>
#     (incremental rebuilds only recompile changed crates)
#   - zig + cargo-zigbuild + rustup target: docker volumes
#     fabro-docker-zig-<arch> / fabro-docker-cargo-tools-<arch>
#   - docker image layers: BuildKit content cache (unchanged binary
#     => cached COPY layers, image build is near-instant)
#   - CLI install is skipped when the staged binary is byte-identical
#
# Requires: mise toolchain (.mise.toml), docker, and docker compose.
#
# Headless shells (no logind session): /run/user/$UID may not exist,
# which breaks just's runtime dir. Export a writable one:
#   export XDG_RUNTIME_DIR="$HOME/.cache/just-run"

set shell := ["bash", "-euo", "pipefail", "-c"]

arch := if arch() == "x86_64" { "amd64" } else if arch() == "aarch64" { "arm64" } else { error("unsupported host arch: " + arch()) }
image := "ghcr.io/fabro-sh/fabro:local"
port := env_var_or_default("FABRO_PORT", "32276")
staged := "tmp/docker-context/" + arch + "/fabro"
cli_bin := env_var("HOME") + "/.fabro/bin/fabro"

# List available recipes
default:
    @just --list

# Full pipeline: build binary + image, install CLI, start compose, wait for
# health, smoke-test the routes a user hits (health, SPA index + every
# referenced asset, SPA deep route, CLI API roundtrip). Smoke failure aborts
# with an ALARM block instead of shipping a broken instance.
#
# LOCK (fabro-332e, partial): two overlapping `just up` runs raced the SPA
# dist mirror on 2026-08-25 and shipped an instance whose UI 404'd every
# asset while health stayed green. The lock file (tmp/just-up.lock, held by
# flock on an open fd for the WHOLE pipeline) makes the second run abort
# immediately with the holder's pid instead of corrupting the mirror.
up:
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p tmp
    exec 9>tmp/just-up.lock
    if ! flock -n 9; then
        holder=$(cat tmp/just-up.lock.pid 2>/dev/null || echo "unknown pid")
        echo "" >&2
        echo "╔══ ALARM: another 'just up' is already running ($holder) ══╗" >&2
        echo "║ A second pipeline would race bun build + the SPA dist mirror ║" >&2
        echo "║ and can ship an instance with a dead UI (2026-08-25 incident).║" >&2
        echo "║ Wait for the running one, or kill $holder first.             ║" >&2
        echo "╚══════════════════════════════════════════════════════════════╝" >&2
        exit 1
    fi
    echo $$ > tmp/just-up.lock.pid
    just clean
    just build-image
    just install-cli
    just compose-up
    just wait-healthy
    just smoke
    just clean

# Build the release binary and the local docker image (cached; uses cargo dev docker-build)
build-image: web-deps
    cargo --locked dev docker-build --arch {{ arch }} --tag {{ image }}

# Build only the release binary and stage it (no docker image build)
build-binary: web-deps
    cargo --locked dev docker-build --arch {{ arch }} --compile-only

# Install workspace JS dependencies for the SPA build (bun workspace, root lockfile)
web-deps:
    bun install --frozen-lockfile

# Install the staged binary as the user CLI (~/.fabro/bin/fabro)
install-cli:
    nu scripts/install-cli.nu "{{ staged }}" "{{ cli_bin }}"

# Start the compose stack (recreates the container when the image changed)
compose-up:
    docker compose up -d

# Stop the compose stack
compose-down:
    docker compose down

# Show compose container status
ps:
    docker compose ps

# Follow server logs
logs:
    docker compose logs -f --tail 200

# Wait until the server health endpoint answers (max 90s)
wait-healthy:
    nu scripts/wait-healthy.nu "{{ port }}"

# Smoke check: health, SPA index + every referenced asset, SPA deep route,
# CLI API roundtrip against the running server (scripts/smoke.nu)
smoke: wait-healthy
    nu scripts/smoke.nu "{{ port }}" "{{ cli_bin }}"

# Clean stale host build artifacts from target/ without a full cargo clean.
# Logic lives in scripts/clean-target.nu; see its header for the growth
# mechanics (cargo never GCs; the docker release build uses its own volume).
#
# Modes: stale (default: drop incremental/ dirs unused >= 6h) | sweep
# (additionally cargo-sweep --time 24) | all (full cargo clean).
# Script flags: `nu scripts/clean-target.nu <mode> --dry-run` to preview.
clean mode="stale":
    nu scripts/clean-target.nu "{{ mode }}"
