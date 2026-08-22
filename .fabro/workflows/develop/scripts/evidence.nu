#!/usr/bin/env nu
# Capture review evidence for the read-only reviewer (prompt node, no tools).
# Emits everything the reviewer needs to judge the pass:
# the diff since the run base, working-tree state, and tracker state.
# The oldest run checkpoint's parent is the run base.

let checkpoints = (git log --format="%H" --grep="Fabro-Completed:" | lines)
let base = if ($checkpoints | is-empty) {
    "HEAD"
} else {
    git rev-parse $"($checkpoints | last)^"
}

print "== changed files since run base =="
git diff --stat $base HEAD

print ""
print "== full diff (bounded to 100k chars) =="
let diff = (git diff $base HEAD | str join "\n")
if ($diff | str length) > 100000 {
    print ($diff | str substring 0..99999)
    print "... (diff truncated)"
} else {
    print $diff
}

print ""
print "== working tree =="
git status --short

print ""
print "== tracker state =="
sd list --format json

print ""
print "== evidence complete =="
