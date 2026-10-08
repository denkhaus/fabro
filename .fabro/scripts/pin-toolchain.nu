#!/usr/bin/env nu
# Pin the server-managed toolchain environment to the image of a deployed
# release (fabro-ed65).
#
# The product command (`fabro env pin-toolchain --tag`) verifies the
# DEPLOYED-SERVER side — the server's embedded build sha must equal the tag —
# but nothing checks the IMAGE side. In the fork.8 window the parity gate
# passed while ghcr.io held no manifest for that tag, so the environment
# pointed at an image no run could pull; only a hand repair saved the window.
# This wrapper is the repo-side policy: manifest first, then delegate.
#
#   nu .fabro/scripts/pin-toolchain.nu [TAG] [--dry-run]
#
# TAG defaults to the line tip (.fabro/scripts/line-tip-sha.nu). Inside a
# deploy window pass the DEPLOYED sha explicitly — the parity gate demands
# the server's embedded sha, which moves on as soon as the line gets more
# commits.

const TOOLCHAIN_REPO = 'ghcr.io/denkhaus/fabro-toolchain'

def fail [what: string]: nothing -> nothing {
    print -e $"pin-toolchain: ($what)"
    exit 1
}

def main [tag: string = '', --dry-run] {
    let t = (if ($tag | str length) > 0 { $tag } else {
        let res = (do { ^nu .fabro/scripts/line-tip-sha.nu } | complete)
        if $res.exit_code != 0 { fail $"line-tip-sha failed: ($res.stderr | str trim)" }
        ($res.stdout | str trim)
    })
    if ($t | str length) != 12 { fail $"'($t)' is not a 12-hex sha tag" }
    let image = $"($TOOLCHAIN_REPO):($t)"
    let check = (do { ^docker manifest inspect $image } | complete)
    if $check.exit_code != 0 {
        fail $"refusing — ($image) has no manifest in GHCR; push the toolchain image under this sha first: nu scripts/run-images.nu --push --sha ($t)"
    }
    print $"pin-toolchain: ($image) manifest OK"
    if $dry_run {
        print $"pin-toolchain: dry-run — would run: fabro env pin-toolchain --tag ($t)"
        return
    }
    ^fabro env pin-toolchain --tag $t
}
