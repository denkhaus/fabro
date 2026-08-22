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
if ($checkpoints | is-empty) {
    print "NO RUN BASE — no checkpoint commits found for this run."
    print "The diff below is empty or misleading; treat this evidence as unreliable."
    print ""
}

print "== changed files since run base =="
git diff --stat $base HEAD

print ""
print "== full diff (per-file, 100k char budget) =="
let changed = (git diff --name-only --diff-filter=ACMRTD $base HEAD | lines)
mut budget = 100000
mut shown = 0
for f in $changed {
    let file_diff = (git diff $base HEAD -- $f | str join "\n")
    let cost = ($file_diff | str length)
    if $cost > $budget {
        print $"--- ($f): NOT SHOWN (budget exhausted, ($cost) chars) ---"
        continue
    }
    $budget = ($budget - $cost)
    $shown = ($shown + 1)
    print (sanitize $file_diff)
}
let omitted = (($changed | length) - $shown)
if $omitted > 0 {
    print ""
    print $"($omitted) file(s) omitted beyond the budget — tail unseen; treat approval accordingly."
}

print ""
print "== working tree =="
git status --short

print ""
print "== tracker state =="
sd list --format json

print ""
print "== in-progress seed specs (authoritative) =="
# The reviewer must judge against the seed description itself, not only the
# planner's brief. Capture the full spec of every in_progress seed.
let ids = (sd list --status in_progress --format json | from json | get issues.id)
if ($ids | is-empty) {
    print "(no seed in progress)"
} else {
    for id in $ids { sd show $id }
}

print ""
print "== evidence complete =="
