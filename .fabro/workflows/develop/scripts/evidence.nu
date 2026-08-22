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
# Structure: private helpers resolve the facts (run base, numstat grouping,
# in-progress seed), section printers render one evidence section each, and
# `main` wires them in the critical-first order. Output text is contract:
# the reviewer prompt names these sections — change wording there too.

# Dev-loop machinery this repo owns; never part of a seed's review scope.
const LOOP_PREFIXES = [".fabro/" ".mulch/" ".seeds/" ".prime/" "docs/" "scripts/"]
const LOOP_ROOTS = ["justfile" "AGENTS.md" "CONTEXT.md" ".gitignore" "lefthook.yml" "go.sum"]

# ---------------------------------------------------------------------------
# helpers — resolve facts
# ---------------------------------------------------------------------------

# Wrap bare " /word " tokens in backticks: agent nodes treat them as skill
# references ("Unknown skill" crash). Two passes catch consecutive tokens
# (a /b /c d) — the trailing space of match one is the leading of match two.
def sanitize [text: string]: nothing -> string {
    let one = ($text | str replace --all --regex '(?m)(^|\s)/([a-z][a-z0-9_.-]*)(\s|$)' '$1`/$2`$3')
    $one | str replace --all --regex '(?m)(^|\s)/([a-z][a-z0-9_.-]*)(\s|$)' '$1`/$2`$3'
}

def current-branch []: nothing -> string {
    git branch --show-current | str trim
}

# Parent of THIS run's oldest checkpoint commit, identified via the run
# branch name (fabro/run/<id> -> subject "fabro(<id>): ..."). The subject
# match is FIXED-STRING: a regex like "fabro.<id>:" silently matches
# nothing (it lacks the ")" before ":"), which once produced empty diff
# sections and misleading evidence. Foreign run checkpoints merged into
# history are ignored. Fallback: HEAD.
def run-base []: nothing -> record<base: string, short: string, grounded: bool> {
    let run_id = (
        current-branch
        | parse --regex 'fabro/run/(?P<id>[^/]+)$'
        | get -o id.0
        | default ''
    )
    let subject_mark = $"fabro\(($run_id)\):"
    let checkpoints = (git log --format=%H --fixed-strings --grep $subject_mark | lines | compact)
    if ($checkpoints | is-empty) {
        {base: "HEAD", short: (git rev-parse --short HEAD), grounded: false}
    } else {
        let base = (git rev-parse $"($checkpoints | last)^")
        {base: $base, short: (git rev-parse --short $base), grounded: true}
    }
}

def is-loop-path [path: string]: nothing -> bool {
    ($LOOP_PREFIXES | any {|p| $path | str starts-with $p}) or ($path in $LOOP_ROOTS)
}

# numstat rows {add del path} for base -> working tree (staged + unstaged).
# numstat emits "-" for binary files; `total` counts those as 0.
def numstat-rows [base: string]: nothing -> list<record<add: string, del: string, path: string>> {
    git diff --numstat $base
    | lines
    | compact
    | each {|l| $l | parse --regex '(?P<add>\d+|-)\s+(?P<del>\d+|-)\s+(?P<path>.+)$' | get 0 }
}

def total [rows: list, col: string]: nothing -> int {
    # Guard the empty case: `math sum` errors on an empty stream (seen with
    # a clean worktree and no run base), and numstat "-" counts as 0.
    let vals = ($rows | get -o $col | default [] | each {|x| if $x == '-' { 0 } else { $x | into int } })
    if ($vals | is-empty) { 0 } else { $vals | math sum }
}

def worktree-state []: nothing -> record<lines: list<string>, label: string> {
    let lines = (git status --porcelain | lines | compact)
    let label = (if ($lines | is-empty) { "clean" } else { $"dirty: ($lines | length) entries" })
    {lines: $lines, label: $label}
}

def in-progress-seed []: nothing -> any {
    sd list --status in_progress --format json
    | from json
    | get -o issues
    | default []
    | get -o 0
    | default null
}

# ---------------------------------------------------------------------------
# section printers — one evidence section each (output text is contract)
# ---------------------------------------------------------------------------

def print-integrity [
    base_short: string
    seed_desc: string
    seed_rows: list
    churn_rows: list
    wt_label: string
]: nothing -> nothing {
    print $"evidence: base=($base_short) seed=($seed_desc)"
    print (integrity-line $seed_rows $churn_rows $wt_label)
}

# The integrity line, for callers that need it (main reprints it at the end).
def integrity-line [
    seed_rows: list
    churn_rows: list
    wt_label: string
]: nothing -> string {
    let seed_files = ($seed_rows | get -o path | default [])
    let churn_files = ($churn_rows | get -o path | default [])
    $"integrity: seed-work=($seed_files | length) files +(total $seed_rows add)/-(total $seed_rows del) | loop-churn=($churn_files | length) files +(total $churn_rows add)/-(total $churn_rows del) | worktree=($wt_label)"
}

def print-seed-spec [wip: any]: nothing -> nothing {
    if $wip != null {
        print ""
        print "== in-progress seed spec (authoritative — judge against this, not the brief) =="
        print $wip.description
    }
}

def print-seed-work-files [seed_rows: list]: nothing -> nothing {
    print ""
    print "== seed work: changed files (review scope — complete diff below) =="
    if ($seed_rows | is-empty) {
        print "(none — no project source changed since run base)"
    } else {
        $seed_rows | each {|r| print $"($r.path) +($r.add)/-($r.del)"}
    }
}

# `complete` keeps stdout as one raw string: the list-of-lines split +
# str join round-trip once injected a stray newline mid-line (observed
# once, never reproduced) — raw capture rules that class out.
def print-seed-work-diff [base: string, seed_rows: list]: nothing -> nothing {
    print ""
    print "== seed work: complete diff (git diff -U1 against run base, files above) =="
    let seed_files = ($seed_rows | get -o path | default [])
    if ($seed_files | is-empty) {
        print "(no seed-work files to diff)"
        return
    }
    let res = (do { git diff -U1 $base -- ...$seed_files } | complete)
    if $res.exit_code != 0 {
        print $"git diff failed: ($res.stderr)"
    } else if ($res.stdout | str length) == 0 {
        print "(empty diff)"
    } else {
        print (sanitize ($res.stdout | str trim -r -c "\n"))
    }
}

def print-loop-churn [churn_rows: list]: nothing -> nothing {
    print ""
    print "== loop churn (dev-loop machinery: workflow/scripts/tracker/expertise/config; counts only, not seed work) =="
    if ($churn_rows | is-empty) {
        print "(none)"
    } else {
        $churn_rows | each {|r| print $"($r.path) +($r.add)/-($r.del)"}
    }
}

def print-worktree [wt_lines: list<string>]: nothing -> nothing {
    print ""
    print "== working tree == git status --porcelain (untracked files show here; they are in NO diff above) =="
    if ($wt_lines | is-empty) {
        print "(clean)"
    } else {
        $wt_lines | each {|l| print (sanitize $l)}
    }
}

# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------

def main []: nothing -> nothing {
    let base = (run-base)
    if not $base.grounded {
        print "NO RUN BASE — no checkpoint commits found for this run."
        print "The diff below is empty or misleading; treat this evidence as unreliable."
        print ""
    }

    let rows = (numstat-rows $base.base)
    let seed_rows = ($rows | where {|r| not (is-loop-path $r.path)} | sort-by path)
    let churn_rows = ($rows | where {|r| is-loop-path $r.path} | sort-by path)

    let wt = (worktree-state)
    let wip = (in-progress-seed)
    let seed_desc = (if $wip == null { "none-in-progress" } else { $"($wip.id): ($wip.title)" })

    # Critical-first order: header, spec, seed-work list + diff, then the
    # tail sections a truncation cuts first.
    print-integrity $base.short $seed_desc $seed_rows $churn_rows $wt.label
    print-seed-spec $wip
    print-seed-work-files $seed_rows
    print-seed-work-diff $base.base $seed_rows
    print-loop-churn $churn_rows
    print-worktree $wt.lines

    # Duplicate of the header line: survives a tail-anchored truncation too.
    print ""
    print (integrity-line $seed_rows $churn_rows $wt.label)
    print "== evidence complete =="
}
