#!/usr/bin/env nu
# The fact head (ADR-0039 stage D, denkhaus/work#1107): line one of a
# reporter fact's text is a deterministic `key=value` machine head
# (`fact=patch-visibility verdict=not-visible base=abc1234
# item=denkhaus/work#1`), prose follows after it. The head is the
# CONTRACT nobody's prose owns: the fast path parses verdicts from it,
# the smoke pins it instead of wording, and an engine retry of the
# closeout re-recording the first fact DEDUPES on the head line (the
# grilled no-dedupe decision's keyed form — append-only stream, the
# consumer collapses).
#
# Grammar: space-separated tokens, each `key=value` with no spaces in
# values; the FIRST token's key is always `fact`. Text whose first line
# does not carry a `fact=` token is prose — parse-head yields null.

# Parse a fact text's machine head: the record of its line-one tokens,
# null when line one carries no `fact=` token (prose-only text).
export def parse-head [text: string]: nothing -> any {
    let first = ($text | lines | get -o 0 | default '')
    if ($first | is-empty) { return null }
    let tokens = ($first | split row ' ' | where {|t| not ($t | str trim | is-empty)})
    if ($tokens | is-empty) { return null }
    if not ($tokens | first | str starts-with 'fact=') { return null }
    mut head = {}
    for token in $tokens {
        let pair = ($token | split row '=' | where {|p| not ($p | is-empty)})
        if ($pair | length) != 2 { return null }
        let key = ($pair | first)
        if ($head | columns | any {|c| $c == $key}) {
            return null
        }
        $head = ($head | insert $key ($pair | last))
    }
    $head
}

# Render a parsed head back to its canonical line: insertion order,
# `key=value` joined by single spaces.
export def head-of [head: record]: nothing -> string {
    $head | items {|key, value| $"($key)=($value)" } | str join ' '
}
