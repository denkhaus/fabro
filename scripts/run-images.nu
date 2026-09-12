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

# One managed image: skip when the built-in label matches the current
# file hash, build otherwise.
def build-one [dockerfile: string, tag: string] {
    if not ($dockerfile | path exists) {
        let name = ($dockerfile | path basename)
        print $"run-images: skip ($tag) \(($name) missing\)"
        return
    }
    let content = (open --raw $dockerfile)
    let hash = ($content | hash sha256)
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
}

def main [] {
    build-one ".fabro/Dockerfile.toolchain" "fabro-toolchain:noble"
    build-one ".fabro/Dockerfile.mise" "fabro-runner:mise"
}
