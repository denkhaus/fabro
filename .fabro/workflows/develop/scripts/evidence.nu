#!/usr/bin/env nu
# Capture review evidence for the read-only reviewer (prompt node, no tools).
#
# PIPE FACTS, measured over four review cycles: at the default `compact`
# fidelity a downstream prompt node renders only the FIRST ~300 characters
# of a command node's output (the render cuts mid-line; nothing after the
# cut survives). The reviewer node therefore sets fidelity="summary:high"
# in workflow.fabro — "detailed summary including outputs" — which carries
# command output in full. This script is sized for that pipe and ordered
# critical-first, so even a truncated render keeps, in order: the integrity
# header, the seed-work file list, and the complete seed-work diff. Loop
# churn counts and worktree state sit at the tail, cut first.
#
# Base detection: parent of THIS run's oldest checkpoint commit, identified
# via the run branch name (fabro/run/<id> -> subject "fabro(<id>): ...").
# The subject match is FIXED-STRING: a regex like "fabro.<id>:" silently
# matches nothing (it lacks the ")" before ":"), which once produced empty
# diff sections and misleading evidence. Foreign run checkpoints merged
# into history are ignored. Fallback: HEAD (the diff is then empty and the
# output says so above the header).
#
# Grouping: changes since base split into SEED WORK (project source files —
# complete diff shown; this is what review verifies) and LOOP CHURN (the
# dev loop's own machinery: workflow, scripts, tracker, expertise, docs,
# config — numstat counts only). Checkpointing is engine-automatic per
# stage, so tooling fixes committed mid-run still land in the run diff;
# quarantining them as counts keeps the review-facing diff exactly the seed
# work. If a seed legitimately targets a churn path, the churn section still
# lists it with its counts — flag it in review and the grouping is fixed
# the next pass.
#
# The diff is taken against the working tree (git diff <base> -- <files>),
# not HEAD, so uncommitted edits are captured too; untracked files are not
# in any diff and surface via the worktree section instead.
#
# The full seed description is NOT duplicated here — the tracker record and
# the Planner's brief carry the spec.
#
# Output hygiene: bare " /word " tokens are wrapped in backticks because
# agent nodes treat them as skill references ("Unknown skill: /tmp" crash).

def sanitize [text: string]: nothing -> string {
    # two passes so consecutive tokens (a /b /c d) are both caught
    let one = ($text | str replace --all --regex '(?m)(^|\s)/([a-z][a-z0-9_.-]*)(\s|$)' '$1`/$2`$3')
    $one | str replace --all --regex '(?m)(^|\s)/([a-z][a-z0-9_.-]*)(\s|$)' '$1`/$2`$3'
}

def total [rows: list, col: string] {
    # numstat emits "-" for binary files; count those as 0
    $rows | get -o $col | default [] | each {|x| if $x == '-' { 0 } else { $x | into int } } | math sum
}

# Dev-loop machinery this repo owns; never part of a seed's review scope here.
let loop_prefixes = [".fabro/" ".mulch/" ".seeds/" ".prime/" "docs/" "scripts/"]
let loop_roots = ["justfile" "AGENTS.md" "CONTEXT.md" ".gitignore" "lefthook.yml" "go.sum"]
let is_loop = {|f|
    ($loop_prefixes | any {|p| $f | str starts-with $p}) or ($f in $loop_roots)
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
let base_short = (git rev-parse --short $base)
if ($checkpoints | is-empty) {
    print "NO RUN BASE — no checkpoint commits found for this run."
    print "The diff below is empty or misleading; treat this evidence as unreliable."
    print ""
}

# numstat rows {add del path} for base -> working tree (staged + unstaged)
let numstat = (
    git diff --numstat $base
    | lines
    | compact
    | each {|l| $l | parse --regex '(?P<add>\d+|-)\s+(?P<del>\d+|-)\s+(?P<path>.+)$' | get 0 }
)
let seed_rows = ($numstat | where {|r| not (do $is_loop $r.path)} | sort-by path)
let churn_rows = ($numstat | where {|r| do $is_loop $r.path} | sort-by path)
let seed_files = ($seed_rows | get -o path | default [])
let churn_files = ($churn_rows | get -o path | default [])

let wt = (git status --porcelain | lines | compact)
let wt_state = (if ($wt | is-empty) { "clean" } else { $"dirty: ($wt | length) entries" })
let wip = (sd list --status in_progress --format json | from json | get -o issues | default [] | get -o 0 | default null)
let seed_desc = (if $wip == null { "none-in-progress" } else { $"($wip.id): ($wip.title)" })

# Integrity header first: survives even the harshest truncation.
let seed_add = (total $seed_rows add)
let seed_del = (total $seed_rows del)
let churn_add = (total $churn_rows add)
let churn_del = (total $churn_rows del)
let integrity = $"integrity: seed-work=($seed_files | length) files +($seed_add)/-($seed_del) | loop-churn=($churn_files | length) files +($churn_add)/-($churn_del) | worktree=($wt_state)"
print $"evidence: base=($base_short) seed=($seed_desc)"
print $integrity

print ""
print "== seed work: changed files (review scope — complete diff below) =="
if ($seed_rows | is-empty) {
    print "(none — no project source changed since run base)"
} else {
    $seed_rows | each {|r| print $"($r.path) +($r.add)/-($r.del)"}
}

print ""
print "== seed work: complete diff (git diff -U1 against run base, files above) =="
if ($seed_files | is-empty) {
    print "(no seed-work files to diff)"
} else {
    # `complete` keeps stdout as one raw string: the list-of-lines split +
    # str join round-trip once injected a stray newline mid-line (observed
    # once, never reproduced) — raw capture rules that class out.
    let res = (do { git diff -U1 $base -- ...$seed_files } | complete)
    if $res.exit_code != 0 {
        print $"git diff failed: ($res.stderr)"
    } else if ($res.stdout | str length) == 0 {
        print "(empty diff)"
    } else {
        print (sanitize ($res.stdout | str trim -r -c "\n"))
    }
}

print ""
print "== loop churn (dev-loop machinery: workflow/scripts/tracker/expertise/config; counts only, not seed work) =="
if ($churn_rows | is-empty) {
    print "(none)"
} else {
    $churn_rows | each {|r| print $"($r.path) +($r.add)/-($r.del)"}
}

print ""
print "== working tree == git status --porcelain (untracked files show here; they are in NO diff above) =="
if ($wt | is-empty) {
    print "(clean)"
} else {
    $wt | each {|l| print (sanitize $l)}
}

print ""
# Duplicate of the header line: survives a tail-anchored truncation too.
print $integrity
print "== evidence complete =="
