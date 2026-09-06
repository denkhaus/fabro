#!/usr/bin/env nu
# Capture review evidence for the reviewer (read-only by capability and
# policy since fabro-1dae; tools available for verification).
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
const LOOP_PREFIXES = [".fabro/" ".mulch/" ".seeds/" ".prime/" ".agents/" "docs/" "scripts/"]
const LOOP_ROOTS = ["justfile" "AGENTS.md" "CLAUDE.md" ".gitignore"]

# Pathological-input safety only — NOT a fidelity budget (see header).
const HARD_CAP = 128000

# Doc-ish extensions sort LAST in the diff walk: review hinges on source.
const DOC_EXTENSIONS = ["md" "markdown" "txt" "rst" "adoc" "ad"]

# ---------------------------------------------------------------------------
# helpers — resolve facts
# ---------------------------------------------------------------------------

# Pipe-safe emission (fabro-ad23): a single `print` of a string larger than
# the pipe buffer (64KiB on Linux) can hit EAGAIN ("Operation would block")
# when the stage reader lags — the 01M1VZTJSZ5 run died exactly there
# printing a 100KiB+ diff. Emit in line-aligned chunks well under the pipe
# capacity so every write fits the buffer; over-long single lines are split
# at character boundaries (nu substring is codepoint-safe, UTF-8 survives).
def emit [text: string]: nothing -> nothing {
    const EMIT_BUDGET = 30000
    if ($text | str length) <= $EMIT_BUDGET {
        print $text
        return
    }
    mut buf = ""
    for line in ($text | lines) {
        mut piece = $line
        # Pathological single line (minified blob): split by characters.
        while ($piece | str length) > $EMIT_BUDGET {
            print --no-newline ($piece | str substring 0..<$EMIT_BUDGET)
            $piece = ($piece | str substring ($EMIT_BUDGET)..)
        }
        if ($buf | str length) + ($piece | str length) + 1 > $EMIT_BUDGET {
            print $buf
            $buf = ""
        }
        $buf = $buf + $piece + "
"
    }
    if ($buf | is-not-empty) {
        print --no-newline $buf
    }
}

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

# The seed's status as recorded in the tracker file at a given commit, or
# null when the file or the seed is absent (or unparsable) there — squashed
# or pre-tracker history degrades to null, never to a wrong status.
def seed-status-at [commit: string, seed_id: string]: nothing -> any {
    let res = (do { git show $"($commit):.seeds/issues.jsonl" } | complete)
    if $res.exit_code != 0 { return null }
    let rows = ($res.stdout | lines | compact | each {|l| do -i { $l | from json } })
    let hit = ($rows | where {|r| $r != null and ($r | get -o id | default '') == $seed_id } | get -o 0 | default null)
    if $hit == null { null } else { $hit | get -o status | default null }
}

# Diff anchor: the commit where the CURRENT seed was claimed — the newest
# commit C in `.seeds/issues.jsonl` history where the seed's status
# TRANSITIONS to in_progress (in_progress at C, not at C^). Transition-
# based, so a re-plan visit (no status change) never re-anchors and every
# earlier cycle of the seed stays visible; and it survives identical
# checkpoint subjects. The claim commit ITSELF is the base, not its
# parent: it carries only tracker churn and loop paths are filtered from
# the diff anyway, so `git diff <claim>` covers exactly the post-claim
# work. Fallback (no claim resolves — squashed history, claim predates
# repo state): the run base, flagged as such — never a silent
# mis-scoped diff.
def seed-claim-base [seed_id: any, run_base: record]: nothing -> record<base: string, short: string, grounded: bool, fallback: bool> {
    if $seed_id != null {
        for c in (git log --format=%H -- .seeds/issues.jsonl | lines | compact) {
            if ((seed-status-at $c $seed_id) == "in_progress") {
                let parent = (do { git rev-parse $"($c)^" } | complete)
                if $parent.exit_code == 0 {
                    let before = (seed-status-at ($parent.stdout | str trim) $seed_id)
                    if $before != "in_progress" {
                        return {base: $c, short: (git rev-parse --short $c), grounded: true, fallback: false}
                    }
                }
            }
        }
    }
    {base: $run_base.base, short: $run_base.short, grounded: $run_base.grounded, fallback: true}
}

# ---------------------------------------------------------------------------
# section printers — one evidence section each (output text is contract)
# ---------------------------------------------------------------------------

def integrity-section [
    base_short: string
    seed_desc: string
    diff_desc: string
    seed_rows: list
    churn_rows: list
    wt_label: string
]: nothing -> string {
    $"(integrity-line $base_short $seed_desc $diff_desc)\n(integrity-line-counts $seed_rows $churn_rows $wt_label)\n"
}

# The header line naming both bases mechanically: the run base (numstat
# and integrity counts stay run-scoped) and the seed-claim base the diff
# section is anchored at. Reprinted at the end so it survives a
# tail-anchored truncation.
def integrity-line [
    base_short: string
    seed_desc: string
    diff_desc: string
]: nothing -> string {
    $"evidence: base=($base_short) seed=($seed_desc) diff-base=($diff_desc)"
}

def integrity-line-counts [
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
# source files first and docs last (diff-sort-key). The base is the
# PER-SEED claim commit (seed-claim-base), not the run base — a capture
# must show only the current seed's hunks, not every closed seed's.
# Files whose per-seed diff is empty are skipped silently: the run-scoped
# file list legitimately names seed-1 leftovers with zero per-seed hunks,
# and emitting them here would only add blank-line noise. HARD_CAP is a
# pathological-input safety only; on cap the walk stops and the
# disclosure names the omitted files — with docs sorted last, a cap hit
# eats documentation before source.
# Shared per-seed diff walk behind both diff sections (seed work and, for
# churn-only seeds, loop work — fabro-4b57): diff each file against the
# claim base, sanitize the body, silently skip files with no hunks, stop at
# the HARD_CAP and report what was cut.
def diff-walk [base: string, files: list]: nothing -> record<body: string, omitted: list> {
    mut used = 0
    mut parts = []
    mut included = []
    for f in $files {
        let res = (do { git diff -U3 $base -- $f } | complete)
        if $res.exit_code != 0 {
            continue
        }
        $included = ($included | append $f)
        let text = (sanitize ($res.stdout | str trim -r -c "\n"))
        if ($text | is-empty) {
            continue
        }
        let cost = ($text | str length)
        if ($used + $cost) > $HARD_CAP { break }
        $used = ($used + $cost)
        $parts = ($parts | append $"($text)\n")
    }
    {body: ($parts | str join), omitted: ($files | where {|f| $f not-in $included })}
}

def diff-section [base: string, seed_rows: list]: nothing -> string {
    let head = "\n== seed work: complete diff (git diff -U3 against the per-seed claim base named in the header, files above; source before docs) ==\n"
    let seed_files = ($seed_rows | get -o path | default [] | sort-by {|f| diff-sort-key $f })
    if ($seed_files | is-empty) {
        return ($head + "(no seed-work files to diff)\n")
    }
    let walk = (diff-walk $base $seed_files)
    if ($walk.omitted | is-empty) {
        $head + $walk.body
    } else {
        $head + $walk.body + "\n(hard cap hit: " + ($walk.omitted | length | into string) + " of " + ($seed_files | length | into string) + " files omitted — treat them as UNSEEN and reject on exact grounds if they matter)\n"
    }
}

# Churn-only dev-loop seeds (seed_rows empty, churn_rows not) used to
# capture as seed-work=0 files +0/-0 with no diff at all, forcing reviewers
# into a manual git detour (fabro-4b57). Same walk, same cap, same
# disclosure as the seed-work section — only the file set and the header
# differ. Emission is gated in `main`: NOT emitted when seed_rows is
# non-empty (loop churn stays counts-only) or when both lists are empty.
def loop-diff-section [base: string, churn_rows: list]: nothing -> string {
    let head = "\n== loop work: complete diff (churn-only seed — loop files diffed with git diff -U3 against the per-seed claim base named in the header; this diff IS the seed's work, source before docs) ==\n"
    let churn_files = ($churn_rows | get -o path | default [] | sort-by {|f| diff-sort-key $f })
    let walk = (diff-walk $base $churn_files)
    if ($walk.omitted | is-empty) and ($walk.body | is-empty) {
        return ($head + "(no loop-work hunks against the claim base)\n")
    }
    if ($walk.omitted | is-empty) {
        $head + $walk.body
    } else {
        $head + $walk.body + "\n(hard cap hit: " + ($walk.omitted | length | into string) + " of " + ($churn_files | length | into string) + " files omitted — treat them as UNSEEN and reject on exact grounds if they matter)\n"
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

    # Journal files (.fabro/journal/<run-id>.jsonl, one line per stage
    # completion, plus legacy <node>@<visit>.json) are meta context:
    # run-machinery stage records about the workflow itself, consumed later
    # by the platform-side improve workflow. They are NO review input —
    # dropped here, so they appear in NO section and NO count.
    let rows = (numstat-rows $base.base | where {|r| not ($r.path | str starts-with ".fabro/journal/")})
    let seed_rows = ($rows | where {|r| not (is-loop-path $r.path)} | sort-by path)
    let churn_rows = ($rows | where {|r| is-loop-path $r.path} | sort-by path)

    let wt = (worktree-state)
    let wip = (in-progress-seed)
    let seed_desc = (if $wip == null { "none-in-progress" } else { $"($wip.id): ($wip.title)" })

    # Diff anchor: the commit where this seed was claimed. Only the diff
    # scope changes — numstat rows, integrity counts, and the file list
    # above stay run-scoped per spec (they are small).
    let claim = (seed-claim-base (if $wip == null { null } else { $wip.id }) $base)
    let diff_desc = (if $claim.fallback {
        $"($claim.short) (NO SEED-CLAIM BASE — no claim transition found in tracker history; diffing against run base)"
    } else {
        $claim.short
    })

    # Fixed sections first, each under its own cap…
    let integrity = (integrity-section $base.short $seed_desc $diff_desc $seed_rows $churn_rows $wt.label)
    let spec = (spec-section $wip)
    let files = (seed-work-files-section $seed_rows)
    let churn = (loop-churn-section $churn_rows)
    # Churn-only seeds get a loop-work diff adjacent to the churn counts
    # (fabro-4b57); mixed or clean captures are unchanged.
    let loop_diff = (if ($seed_rows | is-empty) and (not ($churn_rows | is-empty)) {
        loop-diff-section $claim.base $churn_rows
    } else { "" })
    let worktree = (worktree-section $wt.lines)

    # …then the diff gets whatever remains under the hard output budget
    # (tail sections reserved so a cut lands in the diff, never in them).
    let diff = (diff-section $claim.base $seed_rows)

    # Critical-first emission. No size games: a large capture is the
    # engine's to demote (blobref link) and the agent reviewer's to read.
    emit ($no_base_note | str trim -r -c "\n")
    emit $integrity
    emit $spec
    emit $files
    emit $diff
    emit $churn
    if ($loop_diff | is-not-empty) { emit $loop_diff }
    emit $worktree
    # Duplicate of the header line: survives a tail-anchored truncation too.
    print ""
    print (integrity-line $base.short $seed_desc $diff_desc)
    print "== evidence complete =="
}
