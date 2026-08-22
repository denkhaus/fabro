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

const scope = ".fabro/workflows"

def counterpart [branch: string]: nothing -> string {
    # meta/<name> pairs with <name>; a plain world branch pairs with meta/<name>.
    # Anything deeper (fabro/run/*, fabro/meta/*, ...) is not a world branch.
    let segs = ($branch | split row "/" | length)
    if ($branch | str starts-with "meta/") and $segs == 2 {
        $branch | str replace --regex '^meta/' ''
    } else if $segs == 1 {
        $"meta/($branch)"
    } else {
        ''
    }
}

let here = (git branch --show-current | str trim)
if ($here | is-empty) {
    print "sync-check: detached HEAD — skip"
    exit 0
}
let other = (counterpart $here)
if ($other | is-empty) {
    print "sync-check: no world pairing — skip"
    exit 0
}

# counterpart must exist locally or on origin (fetch it otherwise)
let has_local = (git rev-parse --verify $"refs/heads/($other)" | complete | get exit_code) == 0
let ref = if $has_local { $other } else {
    git fetch origin $other
    $"origin/($other)"
}
if ((git rev-parse --verify $ref | complete | get exit_code) != 0) {
    print $"sync-check: counterpart branch ($other) not found — skip (single-world repo?)"
    exit 0
}

let drift = (git diff $"($ref)" HEAD -- $scope | lines | length)
if $drift > 0 {
    print $"sync-check: DRIFT — ($scope) differs between ($here) and ($ref)"
    git diff --stat $"($ref)" HEAD -- $scope
    print "re-sync with: git checkout <canonical> -- .fabro/workflows"
    exit 1
}
print $"sync-check: ($scope) in sync with ($ref)"
