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
#       architect, anything. NOTE: fabro ps --json carries
#       status as {"kind": "..."} — a string compare against 'running'
#       silently passes while runs are active (the 2026-09-17 violation).
#   (b) NO live work claim of an ACTIVE run on the line's project (ADR-0032
#       phase 3, e829 grill fork 4, settled 2026-10-21; terminal-aware
#       2026-11-13): a claim blocks only while its run is still working —
#       plain `fabro ps` lists active runs, so a claim whose run is absent
#       from that list belongs to a terminal run and is the RECONCILER's
#       to heal (work_reconciler: "run status is the truth"), never the
#       gate's to wait on. The 2026-11-13 incident: a FAILED run's orphan
#       claim held the trunk for an hour with nothing in flight. The
#       merger-era note: awaiting_merge claims of terminal runs no longer
#       block either — the merger rebases base-first and its own gear run
#       is ACTIVE during every sweep, so condition (a) owns that race.
#       A claim whose run cannot be resolved fails the gate CLOSED.
#       An unreachable claims view fails the gate CLOSED.
#
# Exit 0 = GATE OPEN (push allowed). Exit 1 = REFUSED (prints why).
# Usage: nu .fabro/scripts/push-gate.nu [--branch denkhaus]
#        [--server https://mirtuell.net]

def main [
    --branch: string = 'denkhaus'
    --server: string = 'https://mirtuell.net'
] {
    if $branch != 'denkhaus' {
        print $"GATE OPEN: ($branch) is not the line branch — the run gate is denkhaus-scoped \(2026-09-21)"
        exit 0
    }
    # (c) RETIRED 2026-10-26 (cutover stage 5, denkhaus/work#988): the
    # .seeds integrity arm. The tracker moved to work items
    # (denkhaus/work; .seeds deleted in the same change, its archive at
    # denkhaus/work/archive/denkhaus-fabro/) — the 2026-10-14 duplicate-id
    # class is structurally gone: item identity is GitHub's issue number,
    # and the importer's parity report plus the verify gateway own the
    # one-time equivalence proof. Re-checks live in the claims view below.
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
        print $"GATE REFUSED: claims view unreachable on ($server) — the gate fails closed, e829 fork 4"
        exit 1
    }
    let active_ids = ($active | get run_id | each {|id| $id | str substring 0..26})
    let blocking = ($claims | where {|c|
        let state = ($c | get -o state | default '')
        if $state not-in ['granted' 'active' 'awaiting_merge'] { return false }
        # Terminal-aware (2026-11-13): plain ps lists ACTIVE runs only —
        # a claiming run absent from it is terminal, and its claim is the
        # reconciler's to heal, not the gate's to wait on. Unresolvable
        # run ids fail CLOSED (return true blocks below). The claims view
        # is camelCase (runId).
        let claim_run = ($c | get -o runId | default '' | str substring 0..26)
        if ($claim_run | is-empty) { return true }
        $claim_run in $active_ids
    })
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
