#!/usr/bin/env nu
# Build the fork's release image and push it to GHCR
# (`just image-release`). Tags: <version>-<line-tip-sha12> plus `latest`.
#
# Requires a ghcr.io docker login with write:packages:
#   gh auth refresh -s write:packages
#   gh auth token | docker login ghcr.io -u denkhaus --password-stdin

def main [arch: string = "amd64"] {
    let version = (open --raw Cargo.toml | lines
        | where $it =~ '^version = '
        | first
        | parse --regex 'version = "(?P<v>[^"]+)"'
        | get v.0)
    # The pushable LINE-TIP sha (fabro-a1ed/06da): in a GitButler workspace
    # git HEAD is the never-pushed workspace commit, so a tag built from it
    # names a sha no push publishes and `just pin-toolchain`'s parity gate
    # then refuses. ONE policy site: .fabro/scripts/line-tip-sha.nu (it
    # falls back to git HEAD in a plain release clone/CI, and fails closed
    # inside a workspace when `but sha` cannot run).
    let res = (do { ^nu .fabro/scripts/line-tip-sha.nu } | complete)
    if $res.exit_code != 0 {
        error make {msg: $"image-release: line-tip-sha failed: ($res.stderr | str trim)"}
    }
    let sha = ($res.stdout | str trim)
    if ($sha | str length) != 12 {
        error make {msg: $"image-release: line-tip-sha returned '($sha)' — expected a 12-hex sha"}
    }
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
