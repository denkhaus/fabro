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

# Full pipeline: build binary + image, install CLI, start compose, wait for health
up: build-image install-cli compose-up wait-healthy

# Build the release binary and the local docker image (cached; uses cargo dev docker-build)
build-image: web-deps
    cargo --locked dev docker-build --arch {{arch}} --tag {{image}}

# Build only the release binary and stage it (no docker image build)
build-binary: web-deps
    cargo --locked dev docker-build --arch {{arch}} --compile-only

# Install workspace JS dependencies for the SPA build (bun workspace, root lockfile)
web-deps:
    bun install --frozen-lockfile

# Install the staged binary as the user CLI (~/.fabro/bin/fabro)
install-cli:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ ! -x "{{staged}}" ]]; then
        echo "no staged binary at {{staged}} — run 'just build-binary' first" >&2
        exit 1
    fi
    mkdir -p "$(dirname "{{cli_bin}}")"
    if cmp -s "{{staged}}" "{{cli_bin}}"; then
        echo "CLI already up to date: {{cli_bin}}"
    else
        install -m 0755 "{{staged}}" "{{cli_bin}}"
        "{{cli_bin}}" --version
    fi

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
    #!/usr/bin/env bash
    set -euo pipefail
    echo "waiting for http://127.0.0.1:{{port}}/health ..."
    for i in $(seq 1 90); do
        if curl -fsS "http://127.0.0.1:{{port}}/health" >/dev/null 2>&1; then
            echo "healthy after ${i}s"
            exit 0
        fi
        sleep 1
    done
    echo "server did not become healthy within 90s" >&2
    docker compose ps || true
    docker compose logs --tail 50 || true
    exit 1

# Smoke check: health + CLI roundtrip against the running server
smoke: wait-healthy
    "{{cli_bin}}" ps
