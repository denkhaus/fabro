#!/usr/bin/env nu
# The pushable line tip sha12 for release pinning (fabro-a1ed).
#
# In a GitButler workspace, `git rev-parse HEAD` yields the never-pushed
# WORKSPACE commit — the sha a push publishes comes from `but sha <branch>`
# (gitbutler-0cde, fork v0.22.3-fork.3). In a plain checkout (release
# clone, CI), git HEAD IS the pushed tip and stands as-is. Inside a
# GitButler workspace a `but sha` that cannot run fails CLOSED with a
# teaching error — falling back to git HEAD there would silently pin the
# never-pushed workspace commit.
#
# Repo-specific BY DESIGN (loop asset, not core): the line branch name
# lives here, never in the product CLI — the CLI stays generic and takes
# the resolved sha via `--tag`.

# This fork's line branch (ADR-0024 freeze era; the trunk other repos
# track is origin/main).
const LINE_BRANCH = 'denkhaus'

# The sha line of a command's output — some but versions print an
# update-notice line; only a bare hex sha counts.
def sha_line [text: string] {
    $text
    | lines
    | where {|l| ($l | str trim) =~ '^[0-9a-f]{12,40}$' }
    | get 0?
    | default ''
    | str trim
    | str substring 0..11
}

# `but sha <line>` through the repo toolchain first (mise pins the fork
# CLI), then through PATH. Empty string when neither resolves a sha.
def but_sha12 [] {
    let via_mise = (do { ^mise x -- but sha $LINE_BRANCH } | complete)
    if $via_mise.exit_code == 0 {
        let sha = (sha_line $via_mise.stdout)
        if ($sha | str length) == 12 { return $sha }
    }
    let plain = (do { ^but sha $LINE_BRANCH } | complete)
    if $plain.exit_code == 0 {
        let sha = (sha_line $plain.stdout)
        if ($sha | str length) == 12 { return $sha }
    }
    ''
}

# Whether this checkout is a GitButler workspace (a `but` that answers).
def gitbutler_workspace [] {
    let via_mise = (do { ^mise x -- but workspace path } | complete)
    if $via_mise.exit_code == 0 { return true }
    let plain = (do { ^but workspace path } | complete)
    $plain.exit_code == 0
}

def main [] {
    let sha = (but_sha12)
    if ($sha | str length) == 12 {
        print $sha
        return
    }
    if (gitbutler_workspace) {
        print -e $"line-tip-sha: GitButler workspace, but `but sha ($LINE_BRANCH)` did not resolve — update the fork CLI (v0.22.3-fork.3+ has but sha); refusing to fall back to the never-pushed workspace commit"
        exit 1
    }
    let git = (do { ^git rev-parse --short=12 HEAD } | complete)
    if $git.exit_code == 0 {
        print ($git.stdout | str trim)
        return
    }
    print -e 'line-tip-sha: neither but sha nor git rev-parse resolved a sha12'
    exit 1
}
