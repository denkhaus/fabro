#!/usr/bin/env nu
# Capture review evidence for the read-only reviewer (prompt node, no tools).
# Emits everything the reviewer needs to judge the pass: the diff since the
# run base, working-tree state, and tracker state.
#
# Base detection: parent of THIS run's oldest checkpoint commit, identified
# via the run branch name (fabro/run/<id> → subject "fabro(<id>): ...").
# Foreign run checkpoints merged into history are ignored. Fallback: HEAD.
#
# Output hygiene: bare " /word " tokens are wrapped in backticks because
# agent nodes treat them as skill references ("Unknown skill: /tmp" crash).

def sanitize [text: string]: nothing -> string {
    # two passes so consecutive tokens (a /b /c d) are both caught
    let one = ($text | str replace --all --regex '(?m)(^|\s)/([a-z][a-z0-9_.-]*)(\s|$)' '$1`/$2`$3')
    $one | str replace --all --regex '(?m)(^|\s)/([a-z][a-z0-9_.-]*)(\s|$)' '$1`/$2`$3'
}

let branch = (git branch --show-current | str trim)
let run_id = ($branch | parse --regex 'fabro/run/(?P<id>[^/]+)$' | get -o id | default '')
let checkpoints = if ($run_id | is-empty) {
    []
} else {
    git log --format="%H" $"--grep=fabro.($run_id):" | lines
}
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
    print (sanitize ($diff | str substring 0..99999))
    print "... (diff truncated)"
} else {
    print (sanitize $diff)
}

print ""
print "== working tree =="
git status --short

print ""
print "== tracker state =="
sd list --format json

print ""
print "== evidence complete =="
