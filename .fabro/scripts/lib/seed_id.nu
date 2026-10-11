#!/usr/bin/env nu
# The ONE seed-id predicate (fabro-e3c7): every consumer that mints seed
# ids from free text — prompt-lint's resolvable-id check, planner-preflight's
# journal-claim scan, close-claim-check's subject parsing, the salvage
# analysis arms — sources THIS file. No consumer keeps a private regex:
# before this lib, five incompatible copies disagreed (hex-only blind to
# the prefixed family; permissive alnum minting ids from prose like
# 'fabro-workflow').
#
# Accept-set (ONE pair of shapes, matching the tracker's two id forms):
#   fabro-<4 hex>           the bare family
#   fabro-<family>-<4 hex>  the prefixed family (fabro-meta-<hex>)
# Word-boundary guard: a candidate directly followed by another
# identifier char or dash is part of a longer word, never an id.
# Whitelist: prose tokens that share the fabro- prefix but name repo
# surfaces and tools — they must never mint, even if a future shape
# drift would let them match.

# Prose tokens that are repo/tool names, never seed ids.
const PROSE_TOKENS = [
    'fabro-workflow'
    'fabro-validate'
    'fabro-web'
    'fabro-server'
    'fabro-cli'
    'fabro-petri'
    'fabro-dot'
    'fabro-api'
    'fabro-llm'
    'fabro-github'
    'fabro-slack'
    'fabro-mcp-server'
    'fabro-checkpoint'
    'fabro-telemetry'
    'fabro-util'
    'fabro-graphviz'
]

# Every seed id in <text> — full form (fabro- prefix included), unique,
# first-appearance order. The single mint every consumer shares.
export def seed-ids-in [text: string]: nothing -> list<string> {
    let ids = ($text
        | parse --regex 'fabro-(?<id>[0-9a-f]{4}|[a-z][a-z0-9]*-[0-9a-f]{4})(?![0-9A-Za-z-])'
        | get -o id
        | default [])
    if ($ids | is-empty) { return [] }
    $ids
    | each {|id| $"fabro-($id)" }
    | where {|full| $full not-in $PROSE_TOKENS }
    | uniq
}
