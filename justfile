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
# Config persistence across image refreshes (fabro-b03f): the server's
# settings live in the `fabro_fabro-storage` compose volume
# (/storage/.home/settings.toml inside the container). Both compose files
# pin `name: fabro`, so the volume identity is stable no matter which
# directory the stack is brought up from — a routine `just up` refresh
# reuses the configured storage. If the stack ever comes up UNCONFIGURED
# anyway (genuinely fresh volume / first boot / `docker compose down -v`),
# `just smoke` fails with an install-mode alarm; recover with
# `just install-url`, finish the browser wizard, and the container's
# restart policy reboots it configured.
#
# Headless shells (no logind session): /run/user/$UID may not exist,
# which breaks just's runtime dir. Export a writable one:
#   export XDG_RUNTIME_DIR="$HOME/.cache/just-run"

set shell := ["bash", "-euo", "pipefail", "-c"]

arch := if arch() == "x86_64" { "amd64" } else if arch() == "aarch64" { "arm64" } else { error("unsupported host arch: " + arch()) }
image := "ghcr.io/fabro-sh/fabro:local"
port := env("FABRO_PORT", "32276")
staged := "tmp/docker-context/" + arch + "/fabro"
cli_bin := env("HOME") + "/.fabro/bin/fabro"
skip_cli := env("JUST_SKIP_CLI_INSTALL", "false")

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
    just run-images
    just compose-up
    just wait-healthy
    just smoke
    just clean

# Host-side dev-artifact prune (target/ stale GC + tmp/docker-context).
# NEVER touches docker volumes/images/builder cache (30-min toolchain +
# release-build caches live there) — see scripts/dev-artifact-prune.nu.
dev-prune:
    nu scripts/dev-artifact-prune.nu

# Build the release binary and the local docker image (cached; uses cargo dev docker-build)
build-image: web-deps
    cargo --locked dev docker-build --arch {{ arch }} --tag {{ image }}

# Build the release image and push it to the fork's GHCR namespace
# (ghcr.io/denkhaus/fabro): <version>-<shortsha> plus `latest`.
# Needs a ghcr.io docker login with write:packages (see script header).
# Release pipeline (user decision 2026-09-16): the server image AND the
# toolchain image refresh together — the toolchain bakes the fabro CLI
# (af97/fe15; carries the create check since fabro-96c6) that must stay
# in sync with each release; `just up` is no longer the intensive path.
# Also installs the freshly staged binary as the local CLI (fabro-9114):
# the local workflow targets https://mirtuell.net, `just up` (and its
# install-cli step) is no longer used, so THIS recipe keeps local CLI and
# released server at the same version. Escape hatch for pure-CI runs:
# JUST_SKIP_CLI_INSTALL=true just image-release
image-release: web-deps
    nu scripts/image-release.nu "{{ arch }}"
    nu scripts/run-images.nu --push
    @if [ "{{ skip_cli }}" = "true" ]; then \
        echo "image-release: skipping CLI install (JUST_SKIP_CLI_INSTALL=true)"; \
    else \
        just install-cli; \
    fi
    just dev-prune

# Build the run images the lab environments reference (toolchain/mise),
# on demand: rebuilt only when the Dockerfile content hash changed
# (label sh.fabro.toolchain.sha256 carries the built hash).
run-images:
    nu scripts/run-images.nu

# Pin the server-managed toolchain environment to the just-pushed
# ghcr.io/denkhaus/fabro-toolchain:<sha12> tag (thin wrapper over
# `fabro env pin-toolchain --from-run-images`; fabro-4f44). Ordering rule:
# run ONLY after the matching server deploy — the command verifies
# deployed-server/tag parity fail-closed and aborts on mismatch.
pin-toolchain:
    fabro env pin-toolchain --from-run-images

# Build only the release binary and stage it (no docker image build)
build-binary: web-deps
    cargo --locked dev docker-build --arch {{ arch }} --compile-only

# Install workspace JS dependencies for the SPA build (bun workspace, root lockfile)
web-deps:
    bun install --frozen-lockfile

# Install the staged binary as the user CLI (~/.fabro/bin/fabro)
install-cli:
    nu scripts/install-cli.nu "{{ staged }}" "{{ cli_bin }}"

# Start the compose stack (recreates the container when the image changed).
# Both compose files pin `name: fabro` (fabro-b03f), so the storage volume
# holding /storage/.home/settings.toml is reused across refreshes.
compose-up:
    docker compose up -d

# One-command install-mode recovery: print the install URL + token from
# the running container's logs (the server logs the full URL on boot in
# install mode, before /api/v1/* exists). Open it, finish the wizard; the
# container exits and its restart policy reboots it configured, then
# `just smoke` goes green.
install-url:
    #!/usr/bin/env bash
    set -euo pipefail
    url="$(docker compose logs --no-color fabro 2>/dev/null | grep -oE 'https?://[^ ]+/install[?]token=[A-Za-z0-9_-]+' | tail -1 || true)"
    if [ -z "$url" ]; then
        echo "No install URL found in container logs — the server may already be" >&2
        echo "configured, or it is not running. Check: docker compose ps; just logs" >&2
        exit 1
    fi
    echo "$url"

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

# Touched-crates quality gate (fabro-5453): derives changed crates from the
# run diff and gates exactly those (fmt full, clippy+nextest per touched
# crate). Runs inside the run sandbox; logic lives in scripts/qualitygate.nu.
qualitygate:
    nu scripts/qualitygate.nu

# Deterministic verification dispatcher (fabro-6e7f): stage-scoped checks
# derived from the diff — `just verify implementer` is the one mechanical
# verification call agent stages make; the tester gate stays authoritative.
verify stage:
    nu scripts/verify.nu {{ stage }}

# Run a workflow end to end: create+start+attach, wait, integrate the run
# branch (ff-pull when auto-merge landed, else provisional squash-merge).
# No ask-based review — the revisor workflow owns run revisioning
# (ADR-0015). Thin wrapper — logic lives in scripts/run_workflow.nu.
# Examples:
#   just run develop --goal "Implement product seed fabro-6a5a ..."
#   just run develop --adopt 01M0WW
run *args:
    nu scripts/run_workflow.nu {{ args }}

# One ADR-0015 qualification cycle, sequential by design (the
# serialization principle): a develop run (implements the most relevant
# open seed through gate+review), then a revisor run (revises exactly
# that newest run, files seeds with a basis line). Extra args go to the
# DEVELOP leg only (the revisor leg needs no goal). Examples:
#   just cycle
#   just cycle --goal "'implement fabro-6a5a'"
#
# INVOCATION MODEL (user directive 2026-09-05): /iterate and
# /merge-upstream are the USER's manual entry points; this target (and
# `just run`) is the AGENT-facing surface — local agent sessions invoke
# it (background + heartbeat) when an iterate cycle delegates to the
# workflow. Transition path (ADR-0015): manual invocations (by agents
# or the user) until the revisor goes cron on a dedicated 24/7 host.
# While a cycle is in flight, no other tracker consumer (agent session,
# /iterate, upstream merge) may claim seeds — serialization principle:
# one line, one executor.
cycle *args:
    nu scripts/run_workflow.nu develop {{ args }}
    nu scripts/run_workflow.nu revisor

# Validate workflow graphs without the ~7 min Rust test-harness cold
# build: builds just the fabro CLI (fabro-cli subset) and runs the full
# create check (petri admission + the fork's lint rules) on every
# .fabro/workflows/*/workflow.fabro graph. @-file refs are resolved by
# the frontend itself. Optional target: workflow name, workflow dir,
# workflow.toml path, or graph path.
#
# Examples:
#   just validate-workflows
#   just validate-workflows develop
validate-workflows target="":
    nu scripts/validate-workflows.nu {{ if target == "" { "" } else { target } }}

# Lint every nushell script: parse check (nu --ide-check) + the
# interpolated-regex scan that parse tools cannot see (verify.nu class).
lint-nu:
    nu scripts/lint-nu.nu
