#!/usr/bin/env nu
# The pushable line tip sha12 for release pinning (fabro-a1ed).
#
# Plain-git contract (GB era ended 2026-10-09): git HEAD is the line tip.
# The published-tip guard (--require-published, fabro-ed65) is the safety
# net for any checkout that is not a plain branch — a sha that is not the
# tip origin publishes is refused, so a GitButler-era workspace commit (or
# any stale local sha) can never mint a release tag.
#
# HISTORY: through the GB era (2026-10-03..09) resolve probed `but sha`
# first; the first sandbox exercise of the loop battery runner
# (run 01M4H0WWAWX, 2026-10-09) showed the probe CRASHES where but does
# not exist — nu raises command-not-found out of `do { ^but ... } |
# complete`, an error `complete` cannot capture. No but anywhere.
#
# Repo-specific BY DESIGN (loop asset, not core): the line branch name
# lives here, never in the product CLI — the CLI stays generic and takes
# the resolved sha via `--tag`.

# This fork's line branch (ADR-0024 freeze era; the trunk other repos
# track is origin/main).
const LINE_BRANCH = 'denkhaus'

# The verdict of a release tip (PURE — no git, no I/O so a battery can
# pin it): '' when `sha` IS the published tip, else the teaching message.
# Equality, not reachability: GitButler rewrites commit shas on push, so a
# locally-created commit stays an ANCESTOR of the published tip while the
# tip moved on (fork.8 incident: the toolchain image was tagged with the
# local sha of one commit while the fabro image carried the tip).
export def tip-verdict [sha: string, tip: string]: nothing -> string {
    if ($tip | str length) == 0 {
        $"line-tip-sha: cannot read the published tip of origin/($LINE_BRANCH) — refusing to release an unverifiable sha"
    } else if $sha != $tip {
        $"line-tip-sha: ($sha) is not the published tip of origin/($LINE_BRANCH), which is ($tip) — push first; a release tag must name a published tip"
    } else {
        ''
    }
}

# The published tip of the line branch, read from the REMOTE (authoritative;
# a local remote-tracking ref can be stale). Empty string when unreadable.
export def remote-tip []: nothing -> string {
    let res = (do { ^git ls-remote origin $"refs/heads/($LINE_BRANCH)" } | complete)
    if $res.exit_code != 0 { return '' }
    let line = ($res.stdout | lines | get -o 0 | default '')
    let sha = ($line | split row (char tab) | first | default '' | str trim)
    if ($sha | str length) < 12 { return '' }
    ($sha | str substring 0..11)
}

def main [--require-published] {
    let resolved = (resolve)
    if ($resolved | str length) != 12 { exit 1 }
    if $require_published {
        # Release scripts pass this flag: the tag they are about to mint must
        # name the tip a push has PUBLISHED, or the deploy pins an image tag
        # nobody can pull (fabro-ed65).
        let verdict = (tip-verdict $resolved (remote-tip))
        if ($verdict | str length) > 0 {
            print -e $verdict
            exit 1
        }
    }
    print $resolved
}

# The line-tip sha12, or a fail-closed teaching error on stderr.
def resolve []: nothing -> string {
    let git = (do { ^git rev-parse --short=12 HEAD } | complete)
    if $git.exit_code == 0 { return ($git.stdout | str trim) }
    print -e 'line-tip-sha: git rev-parse did not resolve a sha12'
    ''
}
