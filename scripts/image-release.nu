#!/usr/bin/env nu
# Build the fork's release image and push it to GHCR
# (`just image-release`). Tags: <version>-<shortsha> plus `latest`.
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
    let sha = (git rev-parse --short HEAD)
    let repo = "ghcr.io/denkhaus/fabro"
    let tag = $"($repo):($version)-($sha)"

    print $"image-release: building ($tag) \(arch ($arch)) ..."
    cargo --locked dev docker-build --arch $arch --tag $tag
    print $"image-release: tagging ($repo):latest"
    docker tag $tag $"($repo):latest"
    print $"image-release: pushing ($tag) ..."
    docker push $tag
    print $"image-release: pushing ($repo):latest ..."
    docker push $"($repo):latest"
    print $"image-release: pushed ($tag) and ($repo):latest"
}
