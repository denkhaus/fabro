#!/usr/bin/env nu
# Deploy drift (user demand 2026-11-14): is a mirtuell deploy due?
# Compares the live server's build sha against the trunk tip, scoped to
# the deploy-relevant paths — anything that ships inside the binary or
# the SPA. Server-side-only changes (workflows, prompts, docs) ride the
# checkout and need no deploy.
#
#   nu scripts/deploy-drift.nu                    # the verdict
#   nu scripts/deploy-drift.nu --verbose          # + the changed files
#
# Exit 0 always: the verdict is the output, not a gate (callers gate).

# The paths whose changes need a new image: Rust (lib/), the web app
# (apps/fabro-web), and the OpenAPI spec (docs/public/api-reference —
# it regenerates types into the binary and the TS client).
const DEPLOY_RELEVANT = ['lib/' 'apps/fabro-web/' 'docs/public/api-reference/']

def main [
    --server: string = 'https://mirtuell.net',  # the production line
    --verbose (-v),                             # list the changed files
]: nothing -> nothing {
    let version = (do { ^fabro version --server $server --json } | complete)
    if $version.exit_code != 0 {
        print -e $"deploy-drift: fabro version failed: ($version.stderr | str trim | str substring 0..160)"
        return
    }
    let parsed = ($version.stdout | from json)
    let short = ($parsed | get -o server.git_sha | default ($parsed | get -o git_sha) | default '' | str trim)
    if ($short | is-empty) {
        print -e $"deploy-drift: the server named no sha ($parsed | to json -r | str substring 0..120)"
        return
    }
    let tip_full = (do { ^git rev-parse origin/denkhaus } | complete | get stdout | str trim)
    let tip_short = ($tip_full | str substring 0..11)
    if $short == $tip_short {
        print $"deploy-drift: UP TO DATE — live ($short) == tip"
        return
    }
    # Resolve the live short sha to its full form locally (the object is
    # in origin/denkhaus's history after any fetch).
    let live_full = (do { ^git rev-parse $"($short)^{commit}" } | complete)
    if $live_full.exit_code != 0 {
        print $"deploy-drift: DEPLOY LIKELY DUE — live ($short) != tip ($tip_short), and the live sha is not in local history \(fetch first for the file diff\)"
        return
    }
    let live = ($live_full.stdout | str trim)
    let diff = (do { ^git diff --name-only $"($live)..($tip_full)" -- ...$DEPLOY_RELEVANT } | complete)
    let files = ($diff.stdout | lines | where {|l| $l != ''})
    if ($files | is-empty) {
        print $"deploy-drift: NO DEPLOY DUE — tip ($tip_short) is ahead of live ($short), but no deploy-relevant path changed"
        return
    }
    print $"deploy-drift: DEPLOY DUE — ($files | length) deploy-relevant files changed since live ($short):"
    if $verbose {
        $files | each {|f| print $"  ($f)" }
    } else {
        print $"  (rerun with --verbose to list them)"
    }
}
