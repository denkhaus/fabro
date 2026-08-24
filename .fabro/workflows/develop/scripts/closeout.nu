#!/usr/bin/env nu
# Closeout (ADR-0010): deterministic post-approval bookkeeping.
#
# The reviewer's Approved edge lands HERE, not on the planner. This script:
#   1. closes the approved seed (the only close authority besides a human)
#   2. checks the tracker and routes via routing output: preferred_next_label
#      "Tracker empty" (exit) or "More seeds" (planner, fresh claim)
#
# Never an LLM: run 01M0… measured planner@2 at 21s / $0.021 / ~7% of wall
# time for exactly these mechanical actions. Failure keeps the seed open —
# the next run re-enters the review cycle; an approved-but-unclosed seed is
# re-approvable harmlessly.
#
# NOTE (stale verdict): scripts cannot clear context keys. A stale
# review_verdict=approved will still be visible to the planner after
# "More seeds"; the planner prompt explicitly ignores approved verdicts
# (closeout owns them).

def in-progress-seed []: nothing -> any {
    sd list --status in_progress --format json
    | from json
    | get -o issues
    | default []
    | get -o 0
    | default null
}

def main []: nothing -> nothing {
    # The claimed seed IS the in-progress seed (the planner set it on
    # claim; nothing else claims). The tracker is the authoritative source
    # — no context plumbing needed.
    let wip = (in-progress-seed)
    let seed_id = ($wip | get -o id | default "")

    if ($seed_id | is-empty) {
        print -e "closeout: no in-progress seed — nothing to close"
        exit 1
    }

    # Close the approved seed. sd close on an already-closed seed warns but
    # exits 0; that path is unreachable anyway (closeout runs once per
    # approval).
    let res = (do { sd close $seed_id } | complete)
    if $res.exit_code != 0 {
        print -e $"closeout: sd close ($seed_id) failed: ($res.stderr | str trim)"
        exit 1
    }
    print $"closeout: closed ($seed_id)"

    # Tracker check: any open seed left for this effort? A failing sd ready
    # is NOT a closeout failure (the close happened) — route to the planner,
    # which can diagnose tracker state better than an exit can.
    let ready = (do { sd ready --format json } | complete)
    let tracker_empty = (if $ready.exit_code == 0 {
        let issues = ($ready.stdout | from json | get -o issues | default [])
        ($issues | is-empty)
    } else {
        false
    })

    let label = (if $tracker_empty { "Tracker empty" } else { "More seeds" })
    print ({preferred_next_label: $label} | to json --raw)
}
