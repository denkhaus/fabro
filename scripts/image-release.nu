#!/usr/bin/env nu
# Build the fork's release image and push it to GHCR
# (`just image-release`). Tags: <version>-<line-tip-sha12> plus `latest`.
#
# Requires a ghcr.io docker login with write:packages:
#   gh auth refresh -s write:packages
#   gh auth token | docker login ghcr.io -u denkhaus --password-stdin

# `--sha` is how the RELEASE resolves the line tip exactly ONCE (fabro-ed65):
# `just image-release` reads it from .fabro/scripts/line-tip-sha.nu and passes
# the same value to scripts/run-images.nu. Two independent resolutions inside
# one release let a commit landing during the long build tag the fabro image
# and the toolchain image differently, and `just pin-toolchain` then pins an
# image tag nobody ever pushed. Standalone callers omit --sha and get the
# guarded resolution (it refuses a sha that is not the published tip).
def main [arch: string = "amd64", --sha: string = ''] {
    let version = (open --raw Cargo.toml | lines
        | where $it =~ '^version = '
        | first
        | parse --regex 'version = "(?P<v>[^"]+)"'
        | get v.0)
    let tip_sha = (if ($sha | str length) > 0 { $sha } else {
        # ONE policy site: .fabro/scripts/line-tip-sha.nu (fabro-a1ed/06da).
        # In a GitButler workspace git HEAD is the never-pushed workspace
        # commit; --require-published additionally demands the PUBLISHED tip
        # (a release tag must name a sha a pull can find).
        let res = (do { ^nu .fabro/scripts/line-tip-sha.nu --require-published } | complete)
        if $res.exit_code != 0 {
            error make {msg: $"image-release: the published line-tip guard failed: ($res.stderr | str trim)"}
        }
        ($res.stdout | str trim)
    })
    if ($tip_sha | str length) != 12 {
        error make {msg: $"image-release: line tip '($tip_sha)' is not a 12-hex sha"}
    }
    let sha = $tip_sha
    let repo = "ghcr.io/denkhaus/fabro"
    let tag = $"($repo):($version)-($sha)"

    # The SAME line-tip sha goes into the builder as `FABRO_GIT_SHA`
    # (fabro-49af): without it the binary embeds the workspace commit at
    # build time and `just pin-toolchain`'s parity gate refuses a
    # content-identical pair.
    print $"image-release: building ($tag) \(arch ($arch)) ..."
    cargo --locked dev docker-build --arch $arch --tag $tag --git-sha $sha
    print $"image-release: tagging ($repo):latest"
    docker tag $tag $"($repo):latest"
    print $"image-release: pushing ($tag) ..."
    docker push $tag
    print $"image-release: pushing ($repo):latest ..."
    docker push $"($repo):latest"
    print $"image-release: pushed ($tag) and ($repo):latest"
}
