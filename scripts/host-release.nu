#!/usr/bin/env nu
# Build the fork's release image ON THE HOST daemon (ADR-0041 — the
# host-built release path; the git sha is the reproducibility anchor,
# no registry in the critical path).
#
# WHERE THIS RUNS: on the release host (needs docker, nu, git — the
# tofu docker-host module installs all three). `just host-release`
# scp's the CURRENT copy of this script over before invoking it, so the
# executing script never goes stale.
#
# WHAT IT DOES (all heavy work on the host, dev machine only triggers):
#   1. resolve the release sha ONCE, on the host (fabro-ed65 rule):
#      default is the published trunk tip; --sha pins an explicit one
#   2. ensure an anonymous clone of the public repo (no credentials —
#      ADR-0041) and check out that sha
#   3. ensure the release-builder image (docker/release-builder.Dockerfile,
#      content-hash tagged — rebuilds only when the Dockerfile changes)
#   4. run the UNCHANGED pipeline in the builder container against the
#      host daemon: bun install + `cargo dev docker-build`, embedding
#      the sha (--git-sha) exactly like scripts/image-release.nu
#   5. verify the local image and print its tag for tofu
#
# The result is a LOCAL image; the optional ghcr mirror push happens
# AFTER a successful deploy (ADR-0041: default on, driven by fabro-tofu;
# --push-mirror here is the manual escape hatch and needs a ghcr
# write:packages login on the host daemon).

# Anonymous public clone — the repo is public, so no deploy key, no
# token, no ghcr-auth dependency for the source side (ADR-0041).
const SRC_DIR = '/opt/fabro-src'
const REPO_URL = 'https://github.com/denkhaus/fabro.git'
# The local image name: registry-shaped on purpose (tofu consumes it as
# a local tag; a later mirror push needs no re-tag), but never pushed
# by this script.
const IMAGE_REPO = 'denkhaus/fabro'

def main [
    --arch: string = 'amd64'
    --sha: string = ''
    --push-mirror
] {
    let full_sha = (resolve-sha $sha)
    let sha12 = ($full_sha | str substring 0..11)
    print $"host-release: sha ($full_sha) \(($sha12)) arch ($arch)"

    ensure-clone
    checkout $full_sha

    let version = (open --raw $"($SRC_DIR)/Cargo.toml" | lines
        | where $it =~ '^version = '
        | first
        | parse --regex 'version = "(?P<v>[^"]+)"'
        | get v.0)
    let tag = $"($IMAGE_REPO):($version)-($sha12)"

    let builder = (ensure-builder)
    print $"host-release: pipeline start — ($tag)"

    # The builder runs the pipeline unchanged. Named volumes put the
    # cargo registry and the fabro-dev target dir on the host, so the
    # first run pays the fabro-dev compile once; tmp/docker-context is
    # staged inside the mounted clone (gitignored, pruned by the host
    # image sweep).
    let run_args = [
        run
        --rm
        -v "/var/run/docker.sock:/var/run/docker.sock"
        -v $"($SRC_DIR):/src"
        -w /src
        -e "CARGO_TARGET_DIR=/target"
        -v "fabro-release-cargo-registry:/usr/local/cargo/registry"
        -v "fabro-release-target:/target"
        $builder
        sh
        -c
        # NOT -lc: a login shell sources /etc/profile, which RESETS PATH
        # to the Debian default and erases the rust image's ENV
        # /usr/local/cargo/bin — the pipeline died at "cargo: not found"
        # (live 2026-11-09, rehearsal 5). Every tool the pipeline needs
        # is either on the image ENV PATH or /usr/local/bin.
        $"bun install --frozen-lockfile --cwd apps/fabro-web && cargo --locked dev docker-build --arch ($arch) --tag ($tag) --git-sha ($sha12)"
    ]
    let res = (do { ^docker ...$run_args } | complete)
    if $res.exit_code != 0 {
        print -e $res.stderr
        error make {msg: "host-release: the pipeline container failed (log above)"}
    }

    let check = (do { ^docker image inspect $tag } | complete)
    if $check.exit_code != 0 {
        error make {msg: $"host-release: pipeline reported success but \(($tag)) is missing"}
    }
    print $"host-release: local image \(($tag)) ready"

    if $push_mirror {
        # Manual escape hatch only — the default-on mirror is tofu's
        # post-deploy step. Needs a ghcr write login on the host.
        print $"host-release: pushing mirror \(($tag)) ..."
        let push = (do { ^docker push $tag } | complete)
        if $push.exit_code != 0 {
            print -e $push.stderr
            error make {msg: 'host-release: mirror push failed (needs ghcr write:packages login on the host)'}
        }
    }
}

# ONE policy site for the release sha, resolved on the host: default is
# the published trunk tip (anonymous ls-remote — what a pull can find,
# mirroring line-tip-sha's --require-published intent); an explicit
# --sha (12 or 40 hex) pins a review sha and is resolved through the
# clone after fetch.
def resolve-sha [explicit: string]: nothing -> string {
    if ($explicit | str length) > 0 {
        if ($explicit | str length) == 40 { return $explicit }
        if ($explicit | str length) == 12 { return $"($explicit)" }
        error make {msg: $"host-release: --sha must be 12 or 40 hex chars, got '($explicit)'"}
    }
    let res = (do { ^git ls-remote $REPO_URL denkhaus } | complete)
    if $res.exit_code != 0 {
        error make {msg: $"host-release: ls-remote failed: ($res.stderr | str trim)"}
    }
    let row = ($res.stdout | lines | where $it =~ 'refs/heads/denkhaus$' | first)
    let tip = ($row | split column "\t" sha ref | get sha.0)
    if ($tip | str length) != 40 {
        error make {msg: $"host-release: ls-remote returned an unexpected row: ($row)"}
    }
    $tip
}

def ensure-clone [] {
    if ($SRC_DIR | path type) == 'dir' { return }
    print $"host-release: cloning \(($REPO_URL)) → \(($SRC_DIR)) \(one-time)"
    let res = (do { ^git clone $REPO_URL $SRC_DIR } | complete)
    if $res.exit_code != 0 {
        print -e $res.stderr
        error make {msg: 'host-release: clone failed'}
    }
}

def checkout [full_sha: string] {
    let fetch = (do { ^git -C $SRC_DIR fetch origin denkhaus } | complete)
    if $fetch.exit_code != 0 {
        print -e $fetch.stderr
        error make {msg: 'host-release: git fetch failed'}
    }
    # A 12-char --sha resolves to its full form here; anything else
    # must already be full (ls-remote gives 40).
    let rev = (do { ^git -C $SRC_DIR rev-parse $"($full_sha)^{commit}" } | complete)
    if $rev.exit_code != 0 {
        print -e $rev.stderr
        error make {msg: $"host-release: sha \(($full_sha)) not found after fetch"}
    }
    let resolved = ($rev.stdout | str trim)
    let co = (do { ^git -C $SRC_DIR checkout --detach $resolved } | complete)
    if $co.exit_code != 0 {
        print -e $co.stderr
        error make {msg: 'host-release: checkout failed'}
    }
    if $resolved != $full_sha {
        print $"host-release: note — short sha resolved to \(($resolved))"
    }
}

# Content-hash tagged builder image: rebuilds only when the Dockerfile
# changes (the run-images.nu gate-hash pattern, applied to a Dockerfile
# alone).
def ensure-builder []: nothing -> string {
    let hash = (open --raw $"($SRC_DIR)/docker/release-builder.Dockerfile" | hash sha256 | str substring 0..11)
    let tag = $"fabro-release-builder:($hash)"
    let have = (do { ^docker image inspect $tag } | complete)
    if $have.exit_code == 0 { return $tag }
    print $"host-release: building \(($tag))"
    let res = (do {
        ^docker build -f $"($SRC_DIR)/docker/release-builder.Dockerfile" -t $tag $"($SRC_DIR)/docker"
    } | complete)
    if $res.exit_code != 0 {
        print -e $res.stderr
        error make {msg: 'host-release: builder image build failed'}
    }
    $tag
}
