#!/usr/bin/env nu
# Capture review evidence for the reviewer (read-only by policy; tools
# available for verification).
#
# PIPE (agent-reviewer era, run 01M0T2GW): the reviewer is an agent node
# with real tools, and the engine materializes demoted values as blob
# files in the sandbox (.fabro/blobs/<sha>.json) that the reviewer reads
# on demand. Consequence: this capture is COMPLETE — it does not
# self-budget. The old 6.8k self-budget was a workaround for the
# prompt-node reviewer (tool_time_ms 0, blob refs unreadable) and it cut
# exactly the wrong files: run 01M0T2GW reviewer@3 journaled that the
# budget walk kept README.md and omitted main.go + fib_test.go — the
# files the review hinged on.
#
# Size handling is the ENGINE's business now: a large capture is demoted
# to a preview-plus-blobref marker, and the reviewer follows the blobref
# link (its prompt says so). Completeness is THIS script's business:
# every seed-work diff is included, source files before docs, the seed
# spec untruncated. Inputs that arrive as blob refs are resolved inline
# (resolve-blobrefs); unresolvable refs pass through as links rather
# than being dropped.
#
# HARD_CAP is a pathological-input safety only (a generated 50MB file in
# the diff must not produce an unbounded blob): on cap the disclosure
# names what was cut, and source-first ordering guarantees docs go first.
#
# Structure: private helpers resolve the facts (run base, numstat
# grouping, in-progress seed), section printers render one evidence
# section each, and `main` wires them in the critical-first order.
# Output text is contract: the reviewer prompt names these sections —
# change wording there too.

# Dev-loop machinery this repo owns; never part of a seed's review scope.
const LOOP_PREFIXES = [".fabro/" ".mulch/" ".seeds/" ".prime/" "docs/" "scripts/"]
const LOOP_ROOTS = ["justfile" "AGENTS.md" "CONTEXT.md" ".gitignore" "lefthook.yml" "go.sum"]

# Pathological-input safety only — NOT a fidelity budget (see header).
const HARD_CAP = 128000

# Doc-ish extensions sort LAST in the diff walk: review hinges on source.
const DOC_EXTENSIONS = ["md" "markdown" "txt" "rst" "adoc" "ad"]

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

# Resolve `blob://sha256/<hex>` markers in text inputs against the
# engine-materialized blob store (.fabro/blobs/<sha>.json in the sandbox
# workdir). A resolvable ref is inlined verbatim — the capture must carry
# the CONTENT it judges by, not a dead marker. An unresolvable ref (blob
# not materialized here) passes through as the blobref link unchanged:
# downstream (the agent reviewer) can still open it; dropping it would
# lose the only pointer.
def resolve-blobrefs [text: string]: nothing -> string {
    let refs = ($text | parse --regex 'blob://sha256/(?P<sha>[0-9a-f]{64})')
    if ($refs | is-empty) {
        return $text
    }
    mut out = $text
    for sha in ($refs | get sha | uniq) {
        let path = $".fabro/blobs/($sha).json"
        if ($path | path exists) {
            let content = (sanitize (open --raw $path | str trim -r -c "\n"))
            $out = ($out | str replace --all $"blob://sha256/($sha)" $content)
        }
    }
    $out
}

# Source files sort before docs in the diff walk (reviewer@3 painpoint,
# run 01M0T2GW): the complete diff of changed SOURCE files is the primary
# review artifact; docs are context. Stable within each group by path.
def diff-sort-key [path: string]: nothing -> string {
    let ext = ($path | path parse | get extension? | default "" | str lowercase)
    if $ext in $DOC_EXTENSIONS { $"z:($path)" } else { $"a:($path)" }
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
    # COMPLETE, untruncated, blob refs resolved: a cut spec once forced
    # "judge the visible part only" reviews; the engine blob-demotion
    # makes truncation unnecessary (see header).
    let desc = (resolve-blobrefs ($wip.description | str trim -r -c "\n"))
    $head + $desc + "\n"
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
# COMPLETE, no fidelity budget: every seed-work file is included, whole,
# source files first and docs last (diff-sort-key). HARD_CAP is a
# pathological-input safety only; on cap the walk stops and the
# disclosure names the omitted files — with docs sorted last, a cap hit
# eats documentation before source.
def diff-section [base: string, seed_rows: list]: nothing -> string {
    let head = "\n== seed work: complete diff (git diff -U1 against run base, files above; source before docs) ==\n"
    let seed_files = ($seed_rows | get -o path | default [] | sort-by {|f| diff-sort-key $f })
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
        if ($used + $cost) > $HARD_CAP { break }
        $used = ($used + $cost)
        $parts = ($parts | append $"($text)\n")
        $included = ($included | append $f)
    }
    let omitted = ($seed_files | where {|f| $f not-in $included })
    let body = ($parts | str join)
    if ($omitted | is-empty) {
        $head + $body
    } else {
        $head + $body + "\n(hard cap hit: " + ($omitted | length | into string) + " of " + ($seed_files | length | into string) + " files omitted — treat them as UNSEEN and reject on exact grounds if they matter)\n"
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

    # Journal files (.fabro/journal/<node>@<visit>.json) are meta context:
    # run-machinery stage records about the workflow itself, consumed later
    # by the platform-side improve workflow. They are NO review input —
    # dropped here, so they appear in NO section and NO count.
    let rows = (numstat-rows $base.base | where {|r| not ($r.path | str starts-with ".fabro/journal/")})
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
    let diff = (diff-section $base.base $seed_rows)

    # Critical-first emission. No size games: a large capture is the
    # engine's to demote (blobref link) and the agent reviewer's to read.
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
