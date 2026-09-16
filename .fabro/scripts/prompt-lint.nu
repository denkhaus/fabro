#!/usr/bin/env nu
# Prompt-lint for the loop workflows: prompt/graph/toml literals must not
# rot (C3 guard — generalizes fabro-41de's revisor-only check to every
# loop asset).
#
# Checks (.fabro/workflows/{conductor,develop,revisor}/ prompts + graph +
# toml, plus .fabro/scripts/*.nu):
#   1. every `fabro-<hex>{4,}` literal resolves in the tracker (`sd show`)
#      — an unresolvable id is provenance rot. Errors.
#   2. every `justfile:<line>` anchor must point at a NON-comment line —
#      a comment line means the anchor drifted (the planner qualitygate
#      anchor rot class). Errors.
#   3. date pins `20xx-xx-xx` older than 45 days — warnings only (human
#      review; load-bearing pins are allowed to stay).
#
# Exit 1 on any error, 0 otherwise (warnings pass).

def lint-files [] {
    (glob .fabro/workflows/conductor/**/*)
    | append (glob .fabro/workflows/develop/**/*)
    | append (glob .fabro/workflows/revisor/**/*)
    | append (glob .fabro/scripts/*.nu)
    | where {|p| ([$p '.md' '.fabro' '.toml' '.nu'] | any {|ext| ($p | str ends-with $ext)})}
    | where {|p| ($p | path type) == 'file'}
}

def seed-ids-in [text] {
    let matches = ($text | parse --regex 'fabro-(?<id>[0-9a-f]{4,})')
    if ($matches | is-empty) { return [] }
    # word-boundary guard: reject candidates directly followed by another
    # identifier char or dash (a longer non-seed word must not yield a
    # shorter hex-looking prefix)
    $matches.id | uniq | where {|id|
        not ($text | parse --regex ('fabro-' + $id + '(?![0-9a-zA-Z-])') | is-empty)
    }
}

def main [] {
    let files = (lint-files)
    if ($files | is-empty) {
        print -e "prompt-lint: no files found — scope broken"
        exit 2
    }
    mut errors = []
    mut warnings = []

    let justfile_lines = (if ('justfile' | path exists) { open --raw justfile | lines } else { [] })

    for f in $files {
        let text = (open --raw $f)
        # 1. seed ids must resolve
        for id in (seed-ids-in $text) {
            let full = $"fabro-($id)"
            let s = (do { sd show $full --format json } | complete)
            if $s.exit_code != 0 {
                $errors = ($errors | append $"($f): seed id '($full)' does not resolve in the tracker")
            }
        }
        # 2. justfile anchors must not sit on comments
        for m in ($text | parse --regex 'justfile:(?<n>\d+)') {
            let n = ($m.n | into int)
            let line = (if $n < ($justfile_lines | length) { $justfile_lines | get $n } else { null })
            if $line == null {
                $errors = ($errors | append $"($f): justfile:($n) is past end of file")
            } else if (($line | str trim) | str starts-with '#') {
                $errors = ($errors | append $"($f): justfile:($n) points at a comment line — anchor drifted")
            }
        }
        # 3. old date pins warn
        for m in ($text | parse --regex '(?<d>20\d{2}-\d{2}-\d{2})') {
            let dt = ($m.d | into datetime)
            if ((date now) - $dt) > 45day {
                $warnings = ($warnings | append $"($f): date pin '($m.d)' older than 45 days — still load-bearing?")
            }
        }
    }

    for w in ($warnings | uniq) {
        print $"warn: ($w)"
    }
    if ($errors | is-empty) {
        print $"prompt-lint: ok — ($files | length) files, ($warnings | uniq | length) warnings"
    } else {
        for e in ($errors | uniq) {
            print -e $"error: ($e)"
        }
        print -e $"prompt-lint: ($errors | uniq | length) errors"
        exit 1
    }
}
