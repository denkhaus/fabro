#!/usr/bin/env nu
# The ONE seed-id predicate: 'is this a seed id' existed in incompatible
# copies — prompt-lint's hex-only regex was blind to the family shape
# (a fabro-meta-* literal passed unchecked), while planner-preflight's
# wide accept-set minted ids from ordinary repo words ('fabro-workflow'
# became seed id 'workflow'). Every consumer — prompt-lint,
# planner-preflight's journal-claims, close-claim-check, salvage-analysis —
# sources THIS file; none re-implements the shape.
#
# Shapes (one accept-set, both anchored by a left word boundary — no
# identifier char or dash may precede 'fabro-'):
#   short   fabro-[0-9a-z]{4}          the plain tracker id, e.g. fabro-e3c7
#   family  fabro-[a-z]+-[0-9a-f]{4}   the prefixed family, e.g. fabro-meta-c9f2
# A candidate must be the WHOLE word: a tail of further identifier chars
# or dashes is not an id (fabro-e3c7x, fabro-e3c7-suffix both yield []).
# Prose repo words that ride the prefix and survive the shape test are
# whitelisted below; longer/multi-dash words (fabro-workflow,
# fabro-validate, fabro-docker-cargo) are rejected by the shape alone.

# Non-id prose tokens that ride the fabro- prefix AND pass a shape test
# (exactly-four-char placeholders). Longer crate/lane names fail the
# shape test on their own and need no entry here.
const PROSE_TOKENS = [
    'xxxx' # placeholder in docs/batteries (never a real id)
]

def is-seed-token [token: string]: nothing -> bool {
    if ($token | parse --regex '^[0-9a-z]{4}$' | is-not-empty) { return true }
    ($token | parse --regex '^[a-z]+-[0-9a-f]{4}$' | is-not-empty)
}

# Every seed id in <text>, as full ids (fabro-e3c7, fabro-meta-c9f2),
# unique, in first-occurrence order.
export def seed-ids-in [text: string]: nothing -> list<string> {
    $text
    | parse --regex '(?<![0-9a-zA-Z-])fabro-(?<token>[0-9a-z]+(?:-[0-9a-z]+)*)'
    | get -o token
    | default []
    | uniq
    | where {|token| (not ($token in $PROSE_TOKENS)) and (is-seed-token $token) }
    | each {|token| $"fabro-($token)" }
}
