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
# Toolchain coupling (user decision 2026-09-16, fabro-af97; petri rework
# fabro-96c6): the standalone fabro-validate binary is gone — the fork's
# validation rules live in the create check the full CLI carries, so the
# CLI binary hash alone gates the validator surface. `--push` (used by
# `just image-release`) additionally pushes the toolchain image to
# ghcr.io/denkhaus/fabro-toolchain:<git-sha12> — the tag form the
# server-managed environments pin.
#
# Full CLI bake (user decision 2026-09-16, fabro-fe15): the complete
# `fabro` CLI is staged alongside (release profile — the debug binary
# is ~342 MB vs ~147 MB release and the demand is PATH availability,
# not target/debug reuse; runs needing target/debug artifacts still pay
# the cold build, a documented residual). Same gate: the CLI hash joins
# the rebuild decision. Run sandboxes hold no server token (ADR-0019
# note in Dockerfile.toolchain).

# One managed image: skip when the built-in label matches the current
# file hash, build otherwise.
def stage-binary [pkg: string, bin: string] {
    # Build a binary from the current checkout and stage it into the
    # toolchain build context (.fabro/bin/). Small dep subsets; warm
    # builds are seconds, cold builds are one-offs.
    let out_dir = ".fabro/bin"
    if not ($out_dir | path exists) {
        mkdir $out_dir
    }
    let build = (do { cargo build --locked --release --quiet -p $pkg --bin $bin } | complete)
    if $build.exit_code != 0 {
        print -e $build.stderr
        error make {msg: $"run-images: ($bin) build failed"}
    }
    let src = $"target/release/($bin)"
    if not ($src | path exists) {
        error make {msg: $"run-images: ($bin) binary not found after build"}
    }
    cp --force $src $"($out_dir)/($bin)"
    $"($out_dir)/($bin)"
}

# fabro-c643 ARM 1: stage the cargo-chef cook context into the toolchain
# build context (.fabro/cook/) — the git-tracked workspace sources,
# manifests, lockfile, and the OpenAPI spec fabro-api's build.rs reads.
# Chef's planner stage reduces this to recipe.json at build time, so the
# image cook always matches the current checkout's dependency graph.
def stage-cook-context [] {
    let out = ".fabro/cook"
    rm -rf $out
    mkdir $out
    let files = (git ls-files | lines | where {|f|
        ($f | str starts-with "lib/") or ($f | str starts-with "test/") or ($f | str starts-with "docs/public/api-reference/") or ($f == "Cargo.toml") or ($f == "Cargo.lock") or ($f == "rust-toolchain") or ($f == "rust-toolchain.toml") or ($f | str starts-with ".cargo/")
    })
    if ($files | is-empty) {
        error make {msg: "run-images: cook context staging found no files (git ls-files empty?)"}
    }
    $files | each {|f|
        let dest = $"($out)/($f)"
        mkdir ($dest | path dirname)
        cp --force $f $dest
    }
    print $"run-images: staged ($files | length) cook-context files"
    $out
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
        # Petri rework (fabro-96c6): the standalone fabro-validate binary
        # is gone — the CLI carries the create check. Its hash alone gates
        # the validator surface now.
        let cli = (stage-binary fabro-cli fabro)
        let cli_hash = (open --raw $cli | hash sha256)
        # fabro-c643: stage the chef cook context BEFORE hashing, and fold
        # the lockfile hash in — a dependency-graph change must force a
        # toolchain rebuild even when the Dockerfile is byte-identical.
        stage-cook-context
        let lock_hash = (open --raw Cargo.lock | hash sha256)
        ($content | hash sha256) + ($cli_hash | str substring 0..15) + ($lock_hash | str substring 0..15)
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
    # build-one ".fabro/Dockerfile.mise" "fabro-runner:mise" $push
}
