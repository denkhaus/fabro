# The host release runner (ADR-0041): the container that executes the
# release pipeline ON the host daemon. `scripts/host-release.nu` runs
# it with the public clone mounted at its REAL HOST PATH (/opt/fabro-src
# — fabro-dev derives nested docker binds from its workspace root, and
# nested bind sources resolve on the host daemon), the host docker
# socket and CLI mounted in, and the cargo caches on named volumes —
# inside, it runs the unchanged `cargo dev docker-build` orchestration
# (fabro-dev shells out to `docker`, which reaches the host daemon
# through the socket; every container it starts, named volume it
# creates, and layer cache it warms lands on the host).
#
# Contents, each for one pipeline need:
#   rust:1-bookworm  — compiling fabro-dev itself (the same base the
#                      pipeline's inner compile container uses)
#   docker CLI       — copied from the official CLI-only image; the
#                      daemon stays the host's (socket mounted)
#   git, xz-utils    — the sandbox-driver checkout docker-build clones
#                      at the Cargo.lock pin, and tar/untar support
#   bun (+unzip)     — `dev docker-build` runs `spa refresh`, which
#                      shells `bun run build` in apps/fabro-web (the
#                      bun.sh installer needs unzip: the toolchain
#                      Dockerfile hit that on ubuntu already)
FROM rust:1-bookworm

COPY --from=docker:27-cli /usr/local/bin/docker /usr/local/bin/docker

RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends git ca-certificates xz-utils unzip; \
    rm -rf /var/lib/apt/lists/*

RUN set -eux; \
    curl -fsSL https://bun.sh/install | bash -s --; \
    ln -s /root/.bun/bin/bun /usr/local/bin/bun; \
    bun --version; \
    docker --version; \
    git --version
