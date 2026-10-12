#!/usr/bin/env nu
# Seed-id predicate battery: pins BOTH shapes of the shared accept-set in
# lib/seed_id.nu and BOTH failure modes that motivated it —
#   1. short shape: fabro-e3c7-style ids are found (and stay found in
#      every consumer that sources the lib);
#   2. family shape: fabro-meta-c9f2-style ids are found;
#   3. prompt-lint REDS on an UNRESOLVABLE family literal — the old
#      hex-only regex was blind to the whole family;
#   4. planner-preflight's journal-claims minted no id from ordinary
#      repo words: 'the fabro-workflow crate' yields [] — pinned here at
#      the shared predicate journal-claims now calls.
# Boundary cases ride along: id-tailed words (fabro-e3c7x,
# fabro-e3c7-suffix) and the whitelisted fabro-xxxx placeholder yield [].
#
# Synthetic literals are built by concatenation so THIS file carries no
# unresolvable seed-id literal (same discipline as prompt-lint-fixtures).
#
#   nu .fabro/scripts/seed-id-fixtures.nu

const DIR = (path self | path dirname)
const LINT = ($DIR | path join 'prompt-lint.nu')
# synthetic unresolvable family id, split so this battery's own source
# never carries it joined
const DEAD_FAMILY = 'fabro-' + 'meta-beef'
const REAL_FAMILY = 'fabro-' + 'meta-c9f2'

source ($DIR | path join 'lib' 'seed_id.nu')

def fail [what: string]: nothing -> nothing {
    print -e $"seed-id-fixtures: FAIL — ($what)"
    exit 1
}

def expect [got, want, what: string] {
    if ($got != $want) { fail $"($what): got ($got | to json), want ($want | to json)" }
    print $"ok — ($what)"
}

def main [] {
    # 1. short shape
    expect (seed-ids-in 'claim fabro-e3c7 done') ['fabro-e3c7'] 'short shape found'

    # 2. family shape — the whole id, prefix included
    expect (seed-ids-in $"see ($REAL_FAMILY) for detail") [$REAL_FAMILY] 'family shape found as whole id'

    # both shapes side by side, first-occurrence order, unique
    expect (seed-ids-in $"fabro-e3c7 plus ($REAL_FAMILY) plus fabro-e3c7") [
        'fabro-e3c7'
        $REAL_FAMILY
    ] 'both shapes, unique'

    # 3. the preflight failure mode: ordinary repo words never mint ids
    expect (seed-ids-in 'the fabro-workflow crate and fabro-validate script') [] 'prose repo words yield no ids'

    # boundary: a tail of identifier chars or dashes is not an id
    expect (seed-ids-in 'fabro-e3c7x and fabro-e3c7-suffix') [] 'tailed candidates rejected'

    # whitelist: the fabro-xxxx placeholder is not an id
    expect (seed-ids-in 'a fabro-xxxx placeholder') [] 'placeholder token whitelisted'

    # 4. integration: the REAL prompt-lint reds on an unresolvable FAMILY
    # literal in a fresh temp root (the shape the old regex never saw),
    # and stays green on a resolvable family literal.
    let base = (mktemp -d)
    let red_root = ($base | path join 'red')
    mkdir ($red_root | path join '.fabro' 'scripts')
    cp -r ('.seeds' | path expand) ($red_root | path join '.seeds')
    $"# fixture\n# see ($DEAD_FAMILY) for detail\n" | save --force ($red_root | path join '.fabro' 'scripts' 'family-fixtures.nu')
    let red = (do { ^nu -c $"cd '($red_root)'; nu '($LINT)'" } | complete)
    if $red.exit_code == 0 { fail $"prompt-lint stayed green on the unresolvable family literal ($DEAD_FAMILY)" }
    if (not ($red.stdout + $red.stderr | str contains $DEAD_FAMILY)) {
        fail $"prompt-lint red did not name the family literal: ($red.stdout + $red.stderr)"
    }
    print 'ok — prompt-lint reds on an unresolvable family literal'

    let green_root = ($base | path join 'green')
    mkdir ($green_root | path join '.fabro' 'scripts')
    cp -r ('.seeds' | path expand) ($green_root | path join '.seeds')
    $"# fixture\n# see ($REAL_FAMILY) for detail\n" | save --force ($green_root | path join '.fabro' 'scripts' 'family-fixtures.nu')
    let green = (do { ^nu -c $"cd '($green_root)'; nu '($LINT)'" } | complete)
    if $green.exit_code != 0 { fail $"prompt-lint red on the RESOLVABLE family literal ($REAL_FAMILY): ($green.stdout + $green.stderr)" }
    print 'ok — prompt-lint green on a resolvable family literal'

    rm -rf $base
    print 'seed-id-fixtures: ok'
}
