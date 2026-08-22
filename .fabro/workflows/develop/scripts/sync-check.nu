#!/usr/bin/env nu
# Workflow-tree sync check (ADR-0003): .fabro/workflows must be identical
# across the two worlds. Canonical: meta/<name>; synced copy: <name>.
#
# Placement is deliberate: this script lives inside the synced tree, so it
# cannot itself drift. Both worlds' quality gates run it — drift fails the
# gate in whichever world you are working.
#
# Loud skip (exit 0) when the pairing rule does not apply: on run branches
# (fabro/run/*), detached heads, or a world without its counterpart branch.
# The guarantee is enforced on the two world branches, where drift starts.

const SCOPE = ".fabro/workflows"

# ---------------------------------------------------------------------------
# helpers
# ---------------------------------------------------------------------------

def current-branch []: nothing -> string {
    git branch --show-current | str trim
}

# meta/<name> pairs with <name>; a plain world branch pairs with meta/<name>.
# Anything deeper (fabro/run/*, fabro/meta/*, ...) is not a world branch.
def world-counterpart [branch: string]: nothing -> string {
    let segs = ($branch | split row "/" | length)
    let is_meta = ($branch | str starts-with "meta/") and $segs == 2
    let kind = if $is_meta { 'meta' } else if $segs == 1 { 'plain' } else { 'other' }
    match $kind {
        'meta' => ($branch | str replace --regex '^meta/' '')
        'plain' => $"meta/($branch)"
        _ => ''
    }
}

# Resolve the counterpart as a git ref: local branch, else fetched
# origin/<branch>, else null (counterpart does not exist). Fetch output is
# captured and discarded — progress spam must not pollute gate logs.
def counterpart-ref [branch: string]: nothing -> any {
    let local = (do { git rev-parse --verify $"refs/heads/($branch)" } | complete)
    if $local.exit_code == 0 {
        return $branch
    }
    let _ = (do { git fetch origin $branch } | complete)
    let remote = (do { git rev-parse --verify $"origin/($branch)" } | complete)
    if $remote.exit_code == 0 { $"origin/($branch)" } else { null }
}

# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------

def main []: nothing -> nothing {
    let here = (current-branch)
    if ($here | is-empty) {
        print "sync-check: detached HEAD — skip"
        return
    }
    let other = (world-counterpart $here)
    if ($other | is-empty) {
        print "sync-check: no world pairing — skip"
        return
    }
    let ref = (counterpart-ref $other)
    if $ref == null {
        print $"sync-check: counterpart branch ($other) not found — skip (single-world repo?)"
        return
    }

    let drift = (git diff $ref HEAD -- $SCOPE | lines | length)
    if $drift > 0 {
        print $"sync-check: DRIFT — ($SCOPE) differs between ($here) and ($ref)"
        git diff --stat $ref HEAD -- $SCOPE
        print "re-sync with: git checkout <canonical> -- .fabro/workflows"
        exit 1
    }
    print $"sync-check: ($SCOPE) in sync with ($ref)"
}
