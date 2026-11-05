#!/usr/bin/env nu
# The adjudicator closing pass (ADR-0039 stage E, denkhaus/work#1107):
# the LLM-free second half of pass two. The verdict-flipped queue read
# (`closable = enabled`, the merger's settled verdict) is the worklist;
# every entry whose run claimed a work item CLOSES it — merge-complete,
# the claim lands through the door (the adjudicator is the only closer,
# decision 6).
#
# IDEMPOTENCE IS THE DOOR'S OWN ERROR CODES (gap-safe, no prose
# matching — fabro #1118): the door's refusals carry stable codes the
# CLI renders as ` [code]` at the detail's end; the pass matches the
# code, never the prose:
#   - [close_no_live_claim] -> already closed (a previous pass or the
#                              user); done, not a failure.
#   - [close_radius_open]   -> the radius holds open items; the close
#                              waits, the next pass retries.
#   - 200                   -> landed; the item earned its close.
# Any OTHER non-zero exit fails the stage (the standing fact-failure
# discipline: never absorb a failed write).

use ../../../scripts/lib/work_channel.nu [server, work-close-argv, cli, read-queue-hard, emit-envelope]

const CALLER = 'adjudicator-closing-pass'
const FIELD = 'closable'
const VERDICT = 'enabled'
const WORKFLOW = 'develop'

def main [--smoke]: nothing -> nothing {
    let srv = (server $CALLER)
    let entries = (read-queue-hard $CALLER $FIELD $WORKFLOW $VERDICT)
    # Only entries with BOTH decoration halves can close: the work item
    # names the target, the project scopes the close (work#1113 — the
    # project rides the entry, never a hardcode).
    let closable = ($entries | where {|e|
        ($e | get -o work_item | default '') != '' and ($e | get -o project | default '') != ''
    })
    print $"($CALLER): \(($closable | length)\) of them carry a claimable work item"

    mut landed = []
    mut open_radius = []
    mut already_closed = []
    for entry in $closable {
        let run_id = $entry.run.id
        let work_item = $entry.work_item
        let project = $entry.project
        let reason = $"landed via merger \(run ($run_id) closable=enabled\)"
        let res = (cli $CALLER (work-close-argv $srv $project $work_item $reason))
        let stderr = ($res.stderr | str trim)
        if $res.exit_code == 0 {
            $landed = ($landed | append $work_item)
            print $"($CALLER): landed ($work_item) \(run ($run_id)\)"
        } else if ($stderr | str contains '[close_no_live_claim]') {
            $already_closed = ($already_closed | append $work_item)
        } else if ($stderr | str contains '[close_radius_open]') {
            $open_radius = ($open_radius | append $work_item)
        } else {
            print -e $"($CALLER): work close failed for ($work_item): ($stderr | str substring 0..300)"
            exit 1
        }
    }

    print $"($CALLER): landed \(($landed | length)\), radius-open \(($open_radius | length)\) \(retries next pass\), already-closed \(($already_closed | length)\)"
    let closing = {
        landed: $landed,
        radius_open: $open_radius,
        already_closed: $already_closed,
    }
    emit-envelope 'Closing pass done' 'closing' $closing $smoke
}
