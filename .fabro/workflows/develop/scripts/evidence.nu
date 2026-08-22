#!/usr/bin/env nu
# Capture review evidence for the read-only reviewer (prompt node, no tools).
#
# PIPE CONSTRAINT: the engine embeds only the TAIL (~25 lines / ~1KB) of a
# script node's output into downstream prompts — long heads are dropped with
# an "(N lines omitted)" marker the reviewer cannot see past. This script is
# sized and ordered for that pipe:
#   - compact tracker/seed/worktree/changed-file sections first (cut first),
#   - the full diff LAST (what the review verifies against),
#   - a one-line integrity summary as the final line (always survives).
#
# Base detection: parent of THIS run's oldest checkpoint commit, identified
# via the run branch name (fabro/run/<id> → subject "fabro(<id>): ...").
# The subject match is FIXED-STRING: a regex like "fabro.<id>:" silently
# matches nothing (it lacks the ")" before ":"), which once produced empty
# diff sections and misleading evidence. Foreign run checkpoints merged into
# history are ignored. Fallback: HEAD.
#
# The full seed description is NOT duplicated here — the pipe cannot carry
# both it and the diff. The tracker and the Planner's brief carry the spec.
#
# Output hygiene: bare " /word " tokens are wrapped in backticks because
# agent nodes treat them as skill references ("Unknown skill: /tmp" crash).

def sanitize [text: string]: nothing -> string {
    # two passes so consecutive tokens (a /b /c d) are both caught
    let one = ($text | str replace --all --regex '(?m)(^|\s)/([a-z][a-z0-9_.-]*)(\s|$)' '$1`/$2`$3')
    $one | str replace --all --regex '(?m)(^|\s)/([a-z][a-z0-9_.-]*)(\s|$)' '$1`/$2`$3'
}

let branch = (git branch --show-current | str trim)
let run_id = ($branch | parse --regex 'fabro/run/(?P<id>[^/]+)$' | get -o id.0 | default '')
# fixed string, e.g. "fabro(01ABC...):" — parens are literal, dot-regex broke this
let subject_mark = $"fabro\(($run_id)\):"
let checkpoints = (git log --format=%H --fixed-strings --grep $subject_mark | lines | compact)
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

print "== tracker (open seeds) =="
let open = (sd list --format json | from json | get -o issues | default [])
if ($open | is-empty) {
    print "(no open seeds)"
} else {
    $open | each {|i| print $"($i.id) [($i.status)/($i.type)] ($i.title)"}
}

print ""
print "== seed in progress (authoritative spec: tracker record + planner brief) =="
let wip = (sd list --status in_progress --format json | from json | get -o issues | default [])
if ($wip | is-empty) {
    print "(no seed in progress)"
} else {
    $wip | each {|i| print $"($i.id) [($i.status)/($i.type)] ($i.title)"}
}

print ""
print "== changed files since run base (added deleted path) =="
let changed = (git diff --name-only --diff-filter=ACMRTD $base HEAD | lines | compact)
git diff --numstat $base HEAD | lines | compact | each {|l| print $l}

print ""
print "== working tree (incl. ignored files — no binaries should appear) =="
let wt = (git status --short --ignored | lines | compact)
if ($wt | is-empty) {
    print "(clean — no untracked, no ignored leftovers)"
} else {
    $wt | each {|l| print $l}
}

print ""
print "== full diff since run base =="
# Churn (tracker/expertise state) first, source files after — the pipe keeps
# the tail, so review-relevant code lands at the visible end. Per-file line
# counts are printed as banners: whatever slice survives, sizes stay known.
let churn = ($changed | where {|f|
    [".mulch/" ".seeds/" ".prime/" "docs/"] | any {|p| $f | str starts-with $p }
})
let source = ($changed | where {|f| not ($f in $churn)} | sort)
mut total_diff_lines = 0
for f in ($churn | append $source) {
    let file_diff = (git diff $base HEAD -- $f | str join "\n")
    let n = ($file_diff | lines | length)
    $total_diff_lines = ($total_diff_lines + $n)
    print ""
    print $"--- ($f) \(($n) diff lines\) ---"
    if ($file_diff | str length) > 6000 {
        print "NOT SHOWN per-file guard (>6000 chars) — the numstat counts above are the integrity anchor."
    } else {
        print (sanitize $file_diff)
    }
}

print ""
let base_short = (git rev-parse --short $base)
let wt_state = (if ($wt | is-empty) { "clean" } else { $"dirty: ($wt | length) entries" })
let wip_ids = ($wip | get -o id | default [] | str join ",")
print $"summary: base=($base_short) changed=($changed | length) files diff-lines=($total_diff_lines) worktree=($wt_state) in-progress=($wip_ids)"
print "== evidence complete =="
