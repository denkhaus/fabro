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
#   (b) NO open PR on the line repo: run-PRs (fabro/run/* heads, the
#       publish pipeline's vehicles) AND lane-PRs (sibling GitButler lanes'
#       landing vehicles) both block - the serialization is intended
#       (double-landing hazard). The refusal NAMES every blocking PR
#       (number + head), run-PRs and lane-PRs separately (fabro-a9bd).
#
# Exit 0 = GATE OPEN (push allowed). Exit 1 = REFUSED (prints why).
# Usage: nu .fabro/scripts/push-gate.nu [--branch denkhaus]
#        [--server https://mirtuell.net] [--repo denkhaus/fabro]

def main [
    --branch: string = 'denkhaus'
    --server: string = 'https://mirtuell.net'
    --repo: string = 'denkhaus/fabro'
] {
    if $branch != 'denkhaus' {
        print $"GATE OPEN: ($branch) is not the line branch — the run gate is denkhaus-scoped (2026-09-21)"
        exit 0
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
    let pr = (do { ^gh pr list --repo $repo --state open --json number,headRefName } | complete)
    let open = (if $pr.exit_code == 0 {
        $pr.stdout | from json
    } else {
        # gh failing must fail the gate closed, with a visible reason
        [{number: 999, headRefName: 'gh-list-failed'}]
    })
    let run_prs = ($open | where {|p| ($p.headRefName | str starts-with 'fabro/run/') })
    let lane_prs = ($open | where {|p| not ($p.headRefName | str starts-with 'fabro/run/') })
    if ($active | length) > 0 {
        print $"GATE REFUSED: ($active | length) active runs: ($active | each {|r| $r.run_id } | str join ', ')"
        if ($open | length) > 0 { print $"also ($open | length) open PRs" }
        exit 1
    }
    if ($open | length) > 0 {
        if ($run_prs | length) > 0 {
            print $"GATE REFUSED: ($run_prs | length) open run-PRs — after-merge window not reached:"
            for $p in $run_prs { print $"  run-PR  #($p.number) ($p.headRefName)" }
        }
        if ($lane_prs | length) > 0 {
            print $"GATE REFUSED: ($lane_prs | length) open lane-PRs — sibling landing vehicles, serialization intended:"
            for $p in $lane_prs { print $"  lane-PR #($p.number) ($p.headRefName)" }
        }
        exit 1
    }
    print 'GATE OPEN: no active runs, no open PRs — push allowed'
}
