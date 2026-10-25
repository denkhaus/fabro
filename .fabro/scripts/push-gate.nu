#!/usr/bin/env nu
# Push gate for the local iterate/line-watch cycle (user directive
# 2026-09-17: NEVER push to the LINE branch while a pass runs; user
# correction 2026-09-21: the gate is BRANCH-SCOPED — only pushes to the
# line branch denkhaus can dirty a running pass's workspace or its run
# PRs. A push to another branch (a non-line branch, the Petri integration
# branch no run bases on until the W5 cutover) is always OPEN).
#
# Two conditions, both must hold for OPEN on the line branch:
#   (a) NO active run on the production server (status.kind not in the
#       terminal set: succeeded/failed/canceled) — conductor, develop,
#       revisor, architect, anything. NOTE: fabro ps --json carries
#       status as {"kind": "..."} — a string compare against 'running'
#       silently passes while runs are active (the 2026-09-17 violation).
#   (b) NO live work claim on the line's project (ADR-0032 phase 3,
#       e829 grill fork 4, settled 2026-10-21): claims in granted /
#       active / awaiting_merge mean line work is in flight — the
#       awaiting_merge claim's prUrl names the run-PR the merger window
#       will land. Foreign PRs and claimless run-PRs (drained parks'
#       journal vehicles) no longer block: the merger rebases base-first,
#       and gh leaves the gate entirely (the line's own state is the
#       authority). An unreachable claims view fails the gate CLOSED.
#
# Exit 0 = GATE OPEN (push allowed). Exit 1 = REFUSED (prints why).
# Usage: nu .fabro/scripts/push-gate.nu [--branch denkhaus]
#        [--server https://mirtuell.net]

def main [
    --branch: string = 'denkhaus'
    --server: string = 'https://mirtuell.net'
] {
    if $branch != 'denkhaus' {
        print $"GATE OPEN: ($branch) is not the line branch — the run gate is denkhaus-scoped (2026-09-21)"
        exit 0
    }
    # (c) tracker integrity, READ-ONLY: the .seeds JSONL must parse, carry
    # no duplicate ids and no blank lines (2026-10-14 incident: a rebase
    # hand-merge produced a duplicate id + 859 blank lines and downstream
    # JSON parsing stayed green because JSON tolerates surrounding
    # whitespace — the user caught it by eye). Local and cheap, so it runs
    # before the server/PR checks.
    let issues = '.seeds/issues.jsonl'
    if not ($issues | path exists) {
        print $"GATE REFUSED: ($issues) missing — the tracker is part of the line"
        exit 1
    }
    let raw_lines = (open --raw $issues | lines)
    let blank = ($raw_lines | where {|l| $l | str trim | is-empty } | length)
    if $blank > 0 {
        print $"GATE REFUSED: ($blank) blank lines in ($issues) — normalize the tracker before pushing"
        exit 1
    }
    # String-basis id scan (parse/regex only): JSON parse errors would
    # travel as sticky pipeline errors past try/catch in nu, so the gate
    # never asks from json to throw — a line not starting with the id
    # shape IS the corruption signal.
    let recs = ($raw_lines | each {|l| $l | parse --regex '^\{"id":"(?<id>[^"]+)' } | flatten)
    if (($raw_lines | length) != ($recs | length)) {
        print $"GATE REFUSED: ($issues) has records without a leading id — fix the tracker before pushing"
        exit 1
    }
    let dups = ($recs | get id | uniq --count | where count > 1)
    if ($dups | length) > 0 {
        print $"GATE REFUSED: duplicate tracker ids: ($dups | get value | str join ', ') — run seeds dedupe and review the merge"
        exit 1
    }
    let terminal = ['succeeded' 'failed' 'canceled' 'cancelled']
    let ps = (do { ^fabro ps --server $server --json } | complete)
    if $ps.exit_code != 0 {
        print $"GATE REFUSED: fabro ps failed — ($ps.stderr | str substring 0..200)"
        exit 1
    }
    let runs = ($ps.stdout | from json)
    let active = ($runs | where {|r|
        ($r.status?.kind? | default 'unknown') not-in $terminal
    })
    # (b) live claims on the line's project (e829 fork 4): granted /
    # active / awaiting_merge. The auth entry carries `token` for
    # dev-token servers and `access_token` for OAuth (short-lived — the
    # workbench convention); an unreachable claims view fails CLOSED.
    let auth = (open ~/.fabro/auth.json | get servers | get -o $server | default {})
    let token = ($auth | get -o token | default ($auth | get -o access_token | default ''))
    if ($token | is-empty) {
        print $"GATE REFUSED: no auth entry for ($server) in ~/.fabro/auth.json — the claims view is the gate's authority, never skip it"
        exit 1
    }
    let claims = (try {
        http get --headers { Authorization: $"Bearer ($token)" } $"($server)/api/v1/projects/fabro/claims" | get -o data | default []
    } catch {|err|
        null
    })
    if $claims == null {
        print $"GATE REFUSED: claims view unreachable on ($server) — the gate fails closed (e829 fork 4)"
        exit 1
    }
    let blocking = ($claims | where {|c| ($c | get -o state | default '') in ['granted' 'active' 'awaiting_merge'] })
    if ($active | length) > 0 {
        print $"GATE REFUSED: ($active | length) active runs: ($active | each {|r| $r.run_id } | str join ', ')"
        if ($blocking | length) > 0 { print $"also ($blocking | length) live claims" }
        exit 1
    }
    if ($blocking | length) > 0 {
        print $"GATE REFUSED: ($blocking | length) live claims on project fabro — line work in flight:"
        for $c in $blocking {
            let pr = ($c | get -o pr_url | default '')
            let pr_note = (if ($pr | is-empty) { '' } else { ' ' + $pr })
            print $"  claim ($c | get -o claim_id | default '-') ($c | get -o item_key | default '-') [($c | get -o state)]($pr_note)"
        }
        exit 1
    }
    print 'GATE OPEN: no active runs, no live claims — push allowed'
}
