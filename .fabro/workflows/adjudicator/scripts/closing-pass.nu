#!/usr/bin/env nu
# The adjudicator closing pass (ADR-0039 stage E, denkhaus/work#1107):
# the LLM-free second half of pass two. The verdict-flipped queue read
# (`closable = enabled`, the merger's settled verdict) is the worklist;
# every entry whose run claimed a work item CLOSES it — merge-complete,
# the claim lands through the door (the adjudicator is the only closer,
# decision 6).
#
# IDEMPOTENCE IS THE DOOR'S OWN REFUSALS (gap-safe, no extra reads):
#   - 403 "no live claim"        -> already closed (a previous pass or
#                                   the user); done, not a failure.
#   - 409 "open closure radius"  -> the radius holds open items; the
#                                   close waits, the next pass retries.
#   - 200                        -> landed; the item earned its close.
# Any OTHER non-zero exit fails the stage (the standing fact-failure
# discipline: never absorb a failed write).

use ../../../scripts/lib/work_channel.nu [
    server, run-queue-argv, work-close-argv, cli
]

const CALLER = 'adjudicator-closing-pass'
const FIELD = 'closable'
const VERDICT = 'enabled'
const WORKFLOW = 'develop'

# Pure: the queue entries from a run-queue call's stdout.
def entries-from [stdout: string]: nothing -> list<any> {
    let parsed = (try { $stdout | from json } catch { null })
    if ($parsed | describe | str starts-with "record") {
        $parsed | get -o data | default []
    } else { [] }
}

def main [--smoke]: nothing -> nothing {
    let srv = (server $CALLER)
    let queue_res = (cli $CALLER (run-queue-argv $srv $FIELD $WORKFLOW $VERDICT))
    if $queue_res.exit_code != 0 {
        print -e $"($CALLER): closing-pass queue read failed: ($queue_res.stderr | str trim | str substring 0..240)"
        exit 1
    }
    let entries = (entries-from $queue_res.stdout)
    # Only entries with BOTH decoration halves can close: the work item
    # names the target, the project scopes the close (work#1113 — the
    # project rides the entry, never a hardcode).
    let closable = ($entries | where {|e|
        ($e | get -o work_item | default '') != '' and ($e | get -o project | default '') != ''
    })
    print $"($CALLER): \(($entries | length)\) run\(s\) settled on ($FIELD)=($VERDICT); \(($closable | length)\) carry a claimable work item"

    mut landed = []
    mut open_radius = []
    mut already_closed = []
    for entry in $closable {
        let run_id = $entry.run.id
        let work_item = $entry.work_item
        let project = $entry.project
        let reason = $"landed via merger \(run ($run_id) closable=enabled\)"
        let res = (cli $CALLER (work-close-argv $srv $project $work_item $reason))
        if $res.exit_code == 0 {
            $landed = ($landed | append $work_item)
            print $"($CALLER): landed ($work_item) \(run ($run_id)\)"
        } else if ($res.stderr | str contains 'no live claim') {
            $already_closed = ($already_closed | append $work_item)
        } else if ($res.stderr | str contains 'closure radius') {
            $open_radius = ($open_radius | append $work_item)
        } else {
            print -e $"($CALLER): work close failed for ($work_item): ($res.stderr | str trim | str substring 0..300)"
            exit 1
        }
    }

    print $"($CALLER): landed \(($landed | length)\), radius-open \(($open_radius | length)\) \(retries next pass\), already-closed \(($already_closed | length)\)"
    let closing = {
        landed: $landed,
        radius_open: $open_radius,
        already_closed: $already_closed,
    }
    if $smoke {
        $closing | to json --raw | print
        return
    }
    {preferred_next_label: 'Closing pass done', context_updates: {output.closing: $closing}} | to json --raw | print
}
