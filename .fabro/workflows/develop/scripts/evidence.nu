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

# HARD OUTPUT BUDGET: the engine demotes preamble values over 8KB
# (artifact.rs PROMPT_INLINE_VALUE_MAX) to a 300-char preview + blob path a
# tool-less reviewer can never read (painpoint #2 in the mailbox, run
# 01M0NGQXB67674XQ5YCR1MB4BN). This script therefore keeps its ENTIRE
# output under budget: fixed sections render first, the diff gets the
# remainder, files are cut at whole-file boundaries, and every cut is
# disclosed so review can reject on exact grounds. Chars, not bytes: JSON
# escaping inflates newlines/quotes, so the margin below 8192 is deliberate.
const OUTPUT_BUDGET = 6800
const SPEC_CAP = 2200
const TAIL_RESERVE = 700

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

def integrity-section [
    base_short: string
    seed_desc: string
    seed_rows: list
    churn_rows: list
    wt_label: string
]: nothing -> string {
    $"evidence: base=($base_short) seed=($seed_desc)\n(integrity-line $seed_rows $churn_rows $wt_label)\n"
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

def spec-section [wip: any]: nothing -> string {
    if $wip == null { return "" }
    let head = "\n== in-progress seed spec (authoritative — judge against this, not the brief) ==\n"
    let desc = ($wip.description | str trim -r -c "\n")
    if ($desc | str length) > $SPEC_CAP {
        let cut = (($desc | str length) - $SPEC_CAP)
        $head + ($desc | str substring 0..($SPEC_CAP - 1)) + "\n(spec truncated: " + ($cut | into string) + " chars cut — judge the visible part only)\n"
    } else {
        $head + $desc + "\n"
    }
}

def seed-work-files-section [seed_rows: list]: nothing -> string {
    let head = "\n== seed work: changed files (review scope — complete diff below) ==\n"
    if ($seed_rows | is-empty) {
        $head + "(none — no project source changed since run base)\n"
    } else {
        let lines = ($seed_rows | each {|r| $"($r.path) +($r.add)/-($r.del)" } | str join "\n")
        $"($head)($lines)\n"
    }
}

# `complete` keeps stdout as one raw string: the list-of-lines split +
# str join round-trip once injected a stray newline mid-line (observed
# once, never reproduced) — raw capture rules that class out.
# Whole-file budgeting: a file is included completely or not at all; the
# first non-fitting file stops the walk, the rest is disclosed as omitted.
def diff-section [base: string, seed_rows: list, allowance: int]: nothing -> string {
    let head = "\n== seed work: complete diff (git diff -U1 against run base, files above) ==\n"
    let seed_files = ($seed_rows | get -o path | default [])
    if ($seed_files | is-empty) {
        return ($head + "(no seed-work files to diff)\n")
    }
    mut used = 0
    mut parts = []
    mut included = []
    for f in $seed_files {
        let res = (do { git diff -U1 $base -- $f } | complete)
        if $res.exit_code != 0 {
            continue
        }
        let text = (sanitize ($res.stdout | str trim -r -c "\n"))
        let cost = ($text | str length)
        if ($used + $cost) > $allowance { break }
        $used = ($used + $cost)
        $parts = ($parts | append $"($text)\n")
        $included = ($included | append $f)
    }
    let omitted = ($seed_files | where {|f| $f not-in $included })
    let body = ($parts | str join)
    if ($omitted | is-empty) {
        $head + $body
    } else {
        $head + $body + "\n(budget cut: " + ($omitted | length | into string) + " of " + ($seed_files | length | into string) + " files omitted — treat them as UNSEEN and reject on exact grounds if they matter)\n"
    }
}

def loop-churn-section [churn_rows: list]: nothing -> string {
    let head = "\n== loop churn (dev-loop machinery: workflow/scripts/tracker/expertise/config; counts only, not seed work) ==\n"
    if ($churn_rows | is-empty) {
        $head + "(none)\n"
    } else {
        let lines = ($churn_rows | each {|r| $"($r.path) +($r.add)/-($r.del)" } | str join "\n")
        $"($head)($lines)\n"
    }
}

def worktree-section [wt_lines: list<string>]: nothing -> string {
    let head = "\n== working tree == git status --porcelain (untracked files show here; they are in NO diff above) ==\n"
    if ($wt_lines | is-empty) {
        $head + "(clean)\n"
    } else {
        let lines = ($wt_lines | each {|l| sanitize $l } | str join "\n")
        $"($head)($lines)\n"
    }
}

# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------

def main []: nothing -> nothing {
    let base = (run-base)
    let no_base_note = (if not $base.grounded {
        "NO RUN BASE — no checkpoint commits found for this run.\nThe diff below is empty or misleading; treat this evidence as unreliable.\n\n"
    } else { "" })

    let rows = (numstat-rows $base.base)
    let seed_rows = ($rows | where {|r| not (is-loop-path $r.path)} | sort-by path)
    let churn_rows = ($rows | where {|r| is-loop-path $r.path} | sort-by path)

    let wt = (worktree-state)
    let wip = (in-progress-seed)
    let seed_desc = (if $wip == null { "none-in-progress" } else { $"($wip.id): ($wip.title)" })

    # Fixed sections first, each under its own cap…
    let integrity = (integrity-section $base.short $seed_desc $seed_rows $churn_rows $wt.label)
    let spec = (spec-section $wip)
    let files = (seed-work-files-section $seed_rows)
    let churn = (loop-churn-section $churn_rows)
    let worktree = (worktree-section $wt.lines)

    # …then the diff gets whatever remains under the hard output budget
    # (tail sections reserved so a cut lands in the diff, never in them).
    let spent = (($no_base_note | str length) + ($integrity | str length) + ($spec | str length) + ($files | str length))
    let allowance: int = $OUTPUT_BUDGET - $spent - $TAIL_RESERVE
    let diff = (diff-section $base.base $seed_rows $allowance)

    # Critical-first emission; the budget above guarantees the whole
    # capture stays under the engine's 8KB demote threshold.
    print ($no_base_note | str trim -r -c "\n")
    print $integrity
    print $spec
    print $files
    print $diff
    print $churn
    print $worktree
    # Duplicate of the header line: survives a tail-anchored truncation too.
    print ""
    print (integrity-line $seed_rows $churn_rows $wt.label)
    print "== evidence complete =="
}
