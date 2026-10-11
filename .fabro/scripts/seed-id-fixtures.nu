#!/usr/bin/env nu
# seed-id predicate battery (fabro-e3c7): the ONE shared mint
# (.fabro/scripts/lib/seed_id.nu) is the single place seed ids are
# parsed out of prose, so its accept-set is pinned HERE — both id shapes
# (bare hex, prefixed family), the word-boundary guard, uniqueness, and
# the prose-token whitelist. The two failure modes that motivated the lib
# are pinned at their mint source, which every consumer (prompt-lint,
# planner-preflight journal claims, close-claim-check, salvage analysis)
# now sources: 'fabro-workflow' must never yield an id (planner-preflight
# once minted 'workflow' from it), and the prefixed family must yield one
# (prompt-lint's old hex-only regex was blind to it). The prompt-lint
# RED-on-unresolvable-meta arm is pinned by prompt-lint-fixtures.nu.
#
#   nu .fabro/scripts/seed-id-fixtures.nu
#
# Pure: no tracker, no git, no temp roots.

source lib/seed_id.nu

def fail [what: string]: nothing -> nothing {
    print -e $"seed-id-fixtures: FAIL — ($what)"
    exit 1
}

def expect [got, want: list<string>, what: string] {
    if ($got != $want) { fail $"($what): got ($got | to json -r), want ($want | to json -r)" }
}

def main [] {
    # 1. the bare family mints, full form, first-appearance order
    expect (seed-ids-in 'sees fabro-31f7 and fabro-e3c7; fabro-31f7 again') ['fabro-31f7' 'fabro-e3c7'] 'bare ids, order and uniq'

    # 2. the prefixed family mints — the fabro-meta-* shape the old
    #    hex-only prompt-lint regex was blind to
    expect (seed-ids-in 'see fabro-meta-c9f2 for detail') ['fabro-meta-c9f2'] 'prefixed id'
    expect (seed-ids-in 'fabro-meta-c9f2 / fabro-31f7: evidence') ['fabro-meta-c9f2' 'fabro-31f7'] 'both shapes in one line'

    # 3. word-boundary guard: a longer word is never an id
    expect (seed-ids-in 'fabro-e3c7-extra') [] 'trailing dash rejects'
    expect (seed-ids-in 'fabro-meta-c9f2x') [] 'trailing identifier char rejects'
    expect (seed-ids-in 'fabro-31f742') [] 'over-length hex rejects'

    # 4. prose-token whitelist: repo/tool names never mint (the
    #    planner-preflight failure mode — 'fabro-workflow' minted
    #    seed id 'workflow')
    expect (seed-ids-in 'run fabro-workflow, then fabro-validate; fabro-web too') [] 'prose tokens never mint'
    expect (seed-ids-in 'the fabro-server and fabro-cli crates') [] 'crate names never mint'

    # 5. no ids in id-free text
    expect (seed-ids-in 'nothing to see here') [] 'empty'

    print 'seed-id-fixtures: green — 9 assertions'
}
