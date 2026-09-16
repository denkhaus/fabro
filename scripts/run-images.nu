#!/usr/bin/env nu
# Build the run images the lab environments reference, on demand
# (`just run-images`). An image is rebuilt only when its Dockerfile
# content hash changed; the hash rides as an image label, so unchanged
# files are near-instant no-ops.
#
# Pairing (Dockerfile -> tag) mirrors the server-managed environments:
#   .fabro/Dockerfile.toolchain -> fabro-toolchain:noble  (env `toolchain`)
#   .fabro/Dockerfile.mise      -> fabro-runner:mise      (env `mise`)
#
# Called from `just up` before compose-up: the server needs no run image
# itself, but develop runs fail at sandbox create when the referenced
# image is missing — building here keeps `just up` a complete deploy.
#
# Toolchain coupling (user decision 2026-09-16, fabro-af97): the
# toolchain image bakes a fabro-validate binary (validate-only scope —
# no HTTP client, no token surface). This script builds and stages it
# from the current checkout into .fabro/bin/, and the rebuild gate
# hashes Dockerfile content PLUS the binary hash, so every validator
# change forces a toolchain rebuild. `--push` (used by
# `just image-release`) additionally pushes the toolchain image to
# ghcr.io/denkhaus/fabro-toolchain:<git-sha12> — the tag form the
# server-managed environments pin.

# One managed image: skip when the built-in label matches the current
# file hash, build otherwise.
def stage-validator [] {
    # Build the validate-only binary from the current checkout and stage
    # it into the toolchain build context. Small dep subset; warm builds
    # are seconds, the cold build is a one-off.
    let out_dir = ".fabro/bin"
    if not ($out_dir | path exists) {
        mkdir $out_dir
    }
    let build = (do { cargo build --locked --release --quiet -p fabro-validate --bin fabro-validate } | complete)
    if $build.exit_code != 0 {
        print -e $build.stderr
        error make {msg: "run-images: fabro-validate build failed"}
    }
    let src = ("target/release/fabro-validate")
    if not ($src | path exists) {
        error make {msg: "run-images: fabro-validate binary not found after build"}
    }
    cp --force $src $"($out_dir)/fabro-validate"
    $"($out_dir)/fabro-validate"
}

def build-one [dockerfile: string, tag: string, push: bool] {
    if not ($dockerfile | path exists) {
        let name = ($dockerfile | path basename)
        print $"run-images: skip ($tag) \(($name) missing\)"
        return
    }
    let content = (open --raw $dockerfile)
    # Gate hash: Dockerfile content PLUS (toolchain only) the staged
    # validator binary hash — a validator change must force a rebuild
    # even when the Dockerfile is byte-identical.
    let hash = (if $tag == "fabro-toolchain:noble" {
        let bin = (stage-validator)
        let bin_hash = (open --raw $bin | hash sha256)
        ($content | hash sha256) + ($bin_hash | str substring 0..15)
    } else {
        ($content | hash sha256)
    })
    let label = "sh.fabro.toolchain.sha256"
    let wanted = $"($label)=($hash)"
    let inspect = (do {
        ^docker image inspect $tag --format $"{{index .Config.Labels \"($label)\"}}"
    } | complete)
    if $inspect.exit_code == 0 {
        let current = ($inspect.stdout | str trim)
        if $current == $hash {
            print $"run-images: ($tag) up to date \(sha ($hash | str substring 0..11)\)"
            return
        }
    }
    print $"run-images: building ($tag) from ($dockerfile) ..."
    let context = ($dockerfile | path dirname)
    ^docker build --file $dockerfile --tag $tag --label $wanted $context
    print $"run-images: ($tag) built \(sha ($hash | str substring 0..11)\)"
    if $push and $tag == "fabro-toolchain:noble" {
        let sha12 = (git rev-parse --short=12 HEAD | str trim)
        let remote = $"ghcr.io/denkhaus/fabro-toolchain:($sha12)"
        docker tag $tag $remote
        print $"run-images: pushing ($remote) ..."
        docker push $remote
        print $"run-images: pushed ($remote) — server-managed environments pin this sha tag"
    }
}

def main [--push] {
    build-one ".fabro/Dockerfile.toolchain" "fabro-toolchain:noble" $push
    build-one ".fabro/Dockerfile.mise" "fabro-runner:mise" $push
}
