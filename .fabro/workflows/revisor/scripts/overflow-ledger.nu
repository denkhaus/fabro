#!/usr/bin/env nu
# Overflow ledger (fabro-552a): deterministic re-filing of ADR-0022
# overflows. Revision files under `.fabro/revisions/` are the ledger —
# every zero-credit finding is journaled there as a machine-visible
# `- overflow: <title> — <change>; effect: ...` bullet. This script is
# the ONE deterministic consumer surface:
#
#   open     list open (unconsumed) overflow entries as JSON — the
#            revisor file stage merges them into its filing input EVERY
#            pass (deterministic visibility: an unconsumed overflow is
#            never lost to journal rot again; the memoize finding in
#            revision 01M2X8458MDWMRVBRVEMDX9W4J was the motivating loss)
#   consume  mark one entry consumed by appending a `- filed-as: <item
#            key>` line directly after the overflow bullet (idempotent
#            consumption: a second consume of the same entry FAILS, so a
#            filed overflow can never be filed twice)
#   stats    open/consumed counts — REPORT-ONLY surface for the conductor
#            survey; the survey never files and never consumes (the
#            revisor file stage owns filing exclusively)
#
# Production-side dedupe (adjacent-pass rule): before journaling a NEW
# `- overflow:` entry, the file stage checks this listing — same-theme
# open overflow already present means the new revision file records an
# `overflow-dup:` LINK instead, so two adjacent passes journal at most
# ONE open overflow per theme. `overflow-dup:` lines are links, never
# filing input (the parser below does not match them).
#
# Balance semantics are unchanged (ADR-0022): zero-credit handling still
# overflows; this ledger changes VISIBILITY + consumption idempotence
# only, not the filing-balance rule. Backlog-starvation measurement for
# the whole mechanism is open seed fabro-27bb (re-measure revisor
# creates:closes around 2026-09-25).
#
# Canonical shapes (the fixtures battery pins both):
#   open      `- overflow: <title> — <change>; effect: <effect>`
#             (next line NOT a filed-as marker)
#   consumed  the same bullet followed IMMEDIATELY by
#             `- filed-as: <item key> (consumed YYYY-MM-DD[, run <id>])`
#             (written only by `consume` — never hand-edited). Item keys
#             are the tracker's provider-stable form (ADR-0032 phase 3,
#             fork 3); legacy `fabro-xxxx` markers in older revision
#             files still READ as consumed — only the written shape
#             changed.
# Legacy overflow prose without the `- overflow:` bullet is not
# machine-visible and is out of scope.
#
# Usage:
#   nu overflow-ledger.nu open [--dir <revisions-dir>]
#   nu overflow-ledger.nu verify --file <marker.md>
#   nu overflow-ledger.nu stats [--dir <revisions-dir>]
#   nu overflow-ledger.nu consume --file <path> --line <n> --item <item key> [--run-id <id>]
# Exit 0 on success; 2 on bad invocation/usage; 3 on consume of an
# already-consumed entry.

# One entry's title: text up to the first " — " separator, else the
# first 100 chars (the journaling convention puts the title first).
def title-of [rest: string] {
    let parts = ($rest | split row ' — ')
    if ($parts | length) > 1 {
        $parts | first
    } else {
        $rest | str substring 0..<100
    }
}

# Parse every `- overflow:` bullet in one revision file's text.
# consumed == true iff the immediately-following line is a filed-as
# marker (the exact shape `consume` writes) or the rest carries an
# inline filed-as note.
def entries-in [text: string] {
    let lines = ($text | lines)
    let n = ($lines | length)
    if $n == 0 { return [] }
    mut out = []
    for i in 0..($n - 1) {
        let l = ($lines | get $i)
        let m = ($l | parse --regex '^\s*[-*]\s*overflow:\s*(?<rest>\S.*)$')
        if ($m | is-empty) { continue }
        let rest = ($m | first | get rest)
        let nxt = if $i + 1 < $n { $lines | get ($i + 1) } else { '' }
        let consumed = (
            ($nxt | parse --regex '^\s*[-*]\s*filed-as:\s*(fabro-[0-9a-zA-Z_-]+|[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+#[0-9]+)' | is-not-empty)
            or ($rest | str contains 'filed-as: fabro-')
            or ($rest | parse --regex 'filed-as:\s*[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+#[0-9]+' | is-not-empty)
        )
        $out = ($out | append {
            line: ($i + 1)
            title: (title-of $rest)
            text: $rest
            consumed: $consumed
        })
    }
    $out
}

# Collect all entries across a revisions dir, tagged with file basename.
def all-entries [dir: path] {
    if (not ($dir | path expand | path exists)) {
        print -e $"overflow-ledger: revisions dir not found: ($dir)"
        exit 2
    }
    # glob (not ls-with-pattern): nu's ls glob misses hidden-path
    # components (`.fabro/...`) — observed 2026-09-19, fabro-552a.
    let files = (glob ($dir | path join '*.md') | sort)
    $files | each {|f|
        entries-in (open --raw $f) | each {|e| $e | insert file ($f | path basename) }
    } | flatten
}

# Marker shape guard: every finding the marker records as a NEW overflow
# (a `**N. Title** — overflow ...` finding header) must also exist as a
# canonical machine-visible `- overflow:` bullet — prose-only overflow
# findings are invisible to `open` and rot the ledger (the pass-2
# painpoint: one repair commit per malformed marker). `overflow-dup:`
# links are exempt: they link to an existing entry, they do not journal
# one. Exit 0 = shape consistent; exit 1 = mismatch, message names both
# counts and the fix.
def "main verify" [--file: path] {
    if ($file == null or (not ($file | path expand | path exists))) {
        print -e $"overflow-ledger verify: file not found: ($file)"
        exit 2
    }
    let text = (open --raw $file)
    let lines = ($text | lines)
    let claimed = ($lines | where {|l| ($l | parse --regex '^\*\*[0-9]+[a-z]?\.\s.*\*\*\s*—\s*overflow(\s|\(|$)' | is-not-empty)} | length)
    let bullets = ($lines | where {|l| ($l | parse --regex '^\s*[-*]\s*overflow:\s*\S' | is-not-empty)} | length)
    if $claimed != $bullets {
        print -e $"overflow-ledger verify: FAIL — ($file): ($claimed) overflow findings but ($bullets) canonical dash-overflow bullets. Prose-only overflow is not machine-visible: every overflow finding needs its canonical bullet before commit."
        exit 1
    }
    print $"overflow-ledger verify: ok — ($file): ($claimed) overflow findings, ($bullets) canonical bullets"
}

def "main open" [--dir: path = '.fabro/revisions'] {
    all-entries $dir | where consumed == false | to json --raw | print
}

def "main stats" [--dir: path = '.fabro/revisions'] {
    let entries = (all-entries $dir)
    {
        open: ($entries | where consumed == false | length)
        consumed: ($entries | where consumed == true | length)
        files: ($entries | select file | uniq | length)
    } | to json --raw | print
}

# One filed-as marker id: a provider-stable item key (ADR-0032 fork 3,
# e.g. denkhaus/work#123) or a legacy seed id from a pre-migration
# revision file — both READ as consumed; consume WRITES only item keys.
def "main consume" [
    --file: path          # revision file containing the overflow entry
    --line: int           # 1-based line of the '- overflow:' bullet
    --item: string        # item key the entry was filed as
    --run-id: string = '' # invoking run id (optional provenance)
] {
    if ($item | parse --regex '^(fabro-[0-9a-zA-Z_-]+|[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+#[0-9]+)$' | is-empty) {
        let hint = 'expected <owner>/<repo>#<number> or a legacy fabro-xxxx id'
        print -e ('overflow-ledger: bad item key ' + $item + ' — ' + $hint)
        exit 2
    }
    let path = ($file | path expand)
    if (not ($path | path exists)) {
        print -e $"overflow-ledger: file not found: ($file)"
        exit 2
    }
    let lines = (open --raw $path | lines)
    let idx = $line - 1
    if $idx < 0 or $idx >= ($lines | length) {
        print -e $"overflow-ledger: line ($line) out of range in ($file)"
        exit 2
    }
    if (($lines | get $idx) | parse --regex '^\s*[-*]\s*overflow:\s*\S' | is-empty) {
        print -e $"overflow-ledger: line ($line) of ($file) is not an '- overflow:' entry"
        exit 2
    }
    let nxt = if $idx + 1 < ($lines | length) { $lines | get ($idx + 1) } else { '' }
    if ($nxt | parse --regex '^\s*[-*]\s*filed-as:' | is-not-empty) {
        print -e $"overflow-ledger: entry already consumed — ($nxt | str trim)"
        exit 3
    }
    let stamp = (date now | format date '%Y-%m-%d')
    let marker = if ($run_id | is-empty) {
        $"- filed-as: ($item) \(consumed ($stamp)\)"
    } else {
        $"- filed-as: ($item) \(consumed ($stamp), run ($run_id)\)"
    }
    let new = (( $lines | take ($idx + 1) ) ++ [ $marker ] ++ ( $lines | skip ($idx + 1) ))
    # string concat (never `append ''` on a string: nu re-splits the
    # joined string on the separator — observed 2026-09-19, fabro-552a).
    (($new | str join "\n") ++ "\n") | save --force $path
    print $"consumed: ($file):($line) -> ($item)"
}

def main [] {
    print 'usage: overflow-ledger.nu <open|stats|consume> [--dir <dir>] | consume --file <path> --line <n> --item <owner/repo#number> [--run-id <id>]'
    exit 2
}
