#!/usr/bin/env nu
# The develop lane check (denkhaus/work#1144, the sprint-81 start
# stage): BEFORE the planner burns a model call, one script reads
# whether the lane holds ANY open item. An empty lane records the
# lane-drained NOTE — the routed marker the adjudicator fast path
# escalates (denkhaus/work#1146) — and ends the run PR-less; a
# non-empty lane hands over to the planner, which owns the atomic
# claim exactly as before (open-but-claimed items stay the planner's
# work_next answer; this check never claims).
#
# The project and lane arrive as STATIC GRAPH ARGS (ADR-0013: the
# workflow graph is the porting unit that carries repo-specific
# values; the script stays project-agnostic).
#
# FACT FAILURE DISCIPLINE: a failed door call fails the stage.

use ../../../scripts/lib/work_channel.nu [
    server, cli, observation-record, emit-envelope
]

const CALLER = 'develop-lane-check'

def main [
    --project: string  # the work project (static graph arg)
    --lane: string     # the lane this line claims (static graph arg)
    --smoke            # emit the bare record instead of the routing envelope
]: nothing -> nothing {
    let srv = (server $CALLER)
    let res = (cli $CALLER [work '--server' $srv 'list' '--project' $project '--lane' $lane '--limit' '1' '--json'])
    if $res.exit_code != 0 {
        print -e $"($CALLER): work list failed for lane ($lane): ($res.stderr | str trim | str substring 0..240)"
        exit 1
    }
    # The list door prints the BARE item array (open items only — the
    # lane filter is server-side label translation). Nu describes an
    # empty array as list<any> and a record-filled one as table<...> —
    # both are the array shape; anything else is a shape drift.
    let items = (try { $res.stdout | from json } catch { null })
    let shape = ($items | describe)
    if not ($shape | str starts-with 'list') and not ($shape | str starts-with 'table') {
        print -e $"($CALLER): work list stdout is not the bare item array: ($res.stdout | str trim | str substring 0..240)"
        exit 1
    }
    let open_count = ($items | length)
    if $open_count == 0 {
        # The routed marker (work#1146): the fact head's `fact=` token is
        # the class the adjudicator fast path routes on — the note never
        # forces anything by itself.
        let record = (observation-record $CALLER 'note' $"fact=lane-drained lane=($lane)\nthe ($lane) lane holds no open item; no claim attempted — select the next tranche \(work#1144\)")
        if ($record | get -o exit_code | default 1) != 0 {
            print -e $"($CALLER): the lane-drained note failed to record: ($record.stderr | str trim | str substring 0..240)"
            exit 1
        }
        emit-envelope 'Lane drained' 'lane_check' {lane: $lane, project: $project, open_items: 0} $smoke
    } else {
        emit-envelope 'Lane open' 'lane_check' {lane: $lane, project: $project, open_items: $open_count} $smoke
    }
}
