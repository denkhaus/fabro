#!/usr/bin/env nu
# close-claim-check battery (fabro-2a3b): the pure core over the fixture
# pair from the seed — a subject claiming a close whose record keeps its
# old status is a finding; remainder/residual/file phrasings stay quiet.
# `source` is safe: this file defines no main.

use ./close-claim-check.nu [close-claims, close-claim-findings]

def expect [what: string, got, want] {
    if $got != $want {
        print $"RED: ($what)"
        exit 1
    }
    print $"ok: ($what)"
}

def main [] {
    # The parser: claim lists, separators, and the phrasings that are NOT
    # claims (fabro-2a3b's audit evidence: 7 of 8 subjects named other ids
    # in remainder/residual phrasing — only the a9bd case was a lost close).
    expect 'two ids after close' (close-claims 'seeds: close fabro-a9bd + fabro-a1ed') [fabro-a9bd fabro-a1ed]
    expect 'comma list' (close-claims 'seeds+ledger: close fabro-19f9, fabro-f312') [fabro-19f9 fabro-f312]
    expect 'and list' (close-claims 'close fabro-6ac5 and fabro-9c44') [fabro-6ac5 fabro-9c44]
    expect 'single claim with parens' (close-claims 'seeds: close fabro-6ac5 (park any conclusion + no bookkeeping PR)') [fabro-6ac5]
    expect 'reconcile is not a close' (close-claims 'seeds+ledger: close fabro-19f9 + fabro-f312, reconcile fabro-a9bd') [fabro-19f9 fabro-f312]
    expect 'residual phrasing' (close-claims 'seeds: close fabro-1f70 closed as IMPLEMENTED (fabro-986b + 2e7b); residual surfaced as fabro-b869') [fabro-1f70]
    expect 'file phrasing' (close-claims 'seeds: file fabro-74d4 + fabro-19f9 (revisor painpoints)') []
    expect 'no claim' (close-claims 'b869: provider-scoped window gate (fabro-b869 step 1+2+5a)') []

    # The findings: the fixture pair from the incident — 389e53520 claimed
    # a9bd + a1ed, only a1ed landed.
    let subjects = [
        {sha: '701b1f99b', subject: 'a9bd: push-gate names the blocking PRs (fabro-a9bd)'}
        {sha: '389e53520', subject: 'seeds: close fabro-a9bd + fabro-a1ed'}
    ]
    let records = [
        {id: 'fabro-a1ed', status: 'closed'}
        {id: 'fabro-a9bd', status: 'open'}
        {id: 'fabro-74d4', status: 'open'}
    ]
    expect 'lost close is a finding' (close-claim-findings $subjects $records) [[commit seed recorded_status]; ['389e53520' 'fabro-a9bd' 'open']]

    # All claims true -> quiet.
    let records_ok = [
        {id: 'fabro-a1ed', status: 'closed'}
        {id: 'fabro-a9bd', status: 'closed'}
        {id: 'fabro-74d4', status: 'open'}
    ]
    expect 'true claims stay quiet' (close-claim-findings $subjects $records_ok) []

    # A claimed id the tracker cannot resolve is a finding (missing record).
    let records_missing = [{id: 'fabro-a1ed', status: 'closed'}]
    expect 'missing record is a finding' (close-claim-findings $subjects $records_missing) [[commit seed recorded_status]; ['389e53520' 'fabro-a9bd' 'missing']]
    print 'close-claim-check-smoke: green'
}
