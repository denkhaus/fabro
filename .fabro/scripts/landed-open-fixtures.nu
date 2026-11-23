#!/usr/bin/env nu
# Landed-open sweep fixtures: pins the pure half of
# landed-open-sweep.nu (the acceptance chain's pool-hygiene part,
# denkhaus/work#1553) —
#   1. a closable-enabled entry whose item fold reads `closable_landed`
#      IS flagged (integrated, still open — the #1533/#1469/#1535
#      class);
#   2. a waiting entry (not integrated yet) is NOT flagged;
#   3. an entry with no acceptance (no claim) is NOT flagged;
#   4. the flag line names the item key and the integrating run.
#
#   nu .fabro/scripts/landed-open-fixtures.nu

const DIR = (path self | path dirname)

use ($DIR | path join 'landed-open-sweep.nu') [is-landed-open, flag-line]

def fail [what: string]: nothing -> nothing {
    print -e $"landed-open-fixtures: FAIL — ($what)"
    exit 1
}

def expect [got, want, what: string] {
    if ($got != $want) { fail $"($what): got ($got | to json), want ($want | to json)" }
    print $"ok — ($what)"
}

def main [] {
    let landed = {run: {id: '01M82THX28HD7KD4GQS15MVFFB'}, work_item: 'denkhaus/work#1533',
                  acceptance: 'closable_landed'}
    let waiting = {run: {id: '01M82THX28HD7KD4GQS15MVFFB'}, work_item: 'denkhaus/work#1533',
                   acceptance: 'waiting_for_closable'}
    let claimless = {run: {id: '01M82THX28HD7KD4GQS15MVFFB'}, work_item: null, acceptance: null}

    # 1. the landed-but-open class flags
    expect (is-landed-open $landed) true 'closable_landed entry is landed-but-open'

    # 2. not integrated yet — the merger still owes it, not a flag
    expect (is-landed-open $waiting) false 'waiting entry is not flagged'

    # 3. no claim, no acceptance — not a flag
    expect (is-landed-open $claimless) false 'claimless entry is not flagged'

    # 4. the flag line names the item and the run
    expect (flag-line $landed) 'landed-open:denkhaus/work#1533 integrated by 01M82THX28HD7KD4GQS15MVFFB, still open — close or account it' 'flag line names item and run'

    print 'landed-open-fixtures: all pins green'
}
