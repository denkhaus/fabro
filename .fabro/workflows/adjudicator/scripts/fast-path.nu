#!/usr/bin/env nu
# The adjudicator fast path (ADR-0039 stage D part 2, denkhaus/work#1107):
# the LLM-free batch sweep over the adjudication queue. One command stage
# walks the WHOLE worklist — every terminal develop run whose latest
# merge verdict is not settled — marks the trivial mass directly, and
# emits the non-trivial runs as packets for the agent pass (the
# evidence-to-reviewer pipe: scripts fetch, the agent judges).
#
# The trivial case (grilled 2026-11-02): no painpoints (after fact-head
# dedupe — engine retries re-record the first fact; the consumer
# collapses) and an empty closure radius. Notes never trigger anything.
# The verdict splits by pull request: a trivial run WITH a pull request
# is merge: enabled; a trivial run WITHOUT one (a failed or empty run)
# is merge: refused — the verdict means content-mergeable, and nothing
# to merge means refused.
#
# Parked runs re-enter every pass (only enabled and refused free a run):
# they route to the agent pass again — an open escalation is a judgment
# the fast path cannot resolve.
#
# FACT FAILURE DISCIPLINE (user decision 2026-11-01): a door call that
# fails FAILS THE STAGE — exit non-zero, the engine retries; never
# absorb a failed read or write.

use ../../../scripts/lib/fact_head.nu [parse-head, head-of]
use ../../../scripts/lib/work_channel.nu [
    server, run-queue-argv, entries-from, run-mark-argv, observation-list-argv, work-radius-argv, cli
]

const CALLER = 'adjudicator-fast-path'
const FIELD = 'merge'
const WORKFLOW = 'develop'

# Pure: the DISTINCT painpoints of a run's observations — dedupe on the
# machine head (line one): an engine retry of the reporter re-records
# the first fact, and the consumer collapses (the grilled keyed form).
# Notes never trigger; prose without a head counts once by its text.
def distinct-painpoints [observations: list<any>]: nothing -> list<string> {
    let painpoints = ($observations | where {|o| $o.category == "painpoint"})
    mut seen = []
    mut kept = []
    for painpoint in $painpoints {
        let head = (parse-head $painpoint.text)
        let key = (if $head != null { head-of $head } else { $painpoint.text })
        if not ($key in $seen) {
            $seen = ($seen | append $key)
            $kept = ($kept | append $painpoint.text)
        }
    }
    $kept
}

# One run's observation records through the read door (HARD: a failed
# read fails the stage).
def read-observations [run_id: string]: nothing -> list<any> {
    let srv = (server $CALLER)
    let res = (cli $CALLER (observation-list-argv $srv $run_id))
    if $res.exit_code != 0 {
        print -e $"($CALLER): observation list failed for ($run_id): ($res.stderr | str trim | str substring 0..240)"
        exit 1
    }
    let parsed = (try { $res.stdout | from json } catch { null })
    if ($parsed | describe | str starts-with "record") {
        $parsed | get -o observations | default []
    } else { [] }
}

# The work item's closure radius through the read door: the OPEN work
# items linked to the item's observations. [] when the run claims no
# work item. The project rides the queue entry (work#1113) — never a
# hardcoded project id. HARD on transport failure.
def read-radius [project: string, work_item: string]: nothing -> list<any> {
    if ($work_item | is-empty) { return [] }
    let srv = (server $CALLER)
    let res = (cli $CALLER (work-radius-argv $srv $project $work_item))
    if $res.exit_code != 0 {
        print -e $"($CALLER): work radius failed for ($work_item): ($res.stderr | str trim | str substring 0..240)"
        exit 1
    }
    let parsed = (try { $res.stdout | from json } catch { null })
    if ($parsed | describe | str starts-with "record") {
        $parsed | get -o radius | default []
    } else { [] }
}

# One verdict write through the door (HARD: a failed mark fails the
# stage — the engine retries the whole sweep; marks are idempotent at
# the fold, a re-sweep is gap-safe).
def mark-run [run_id: string, verdict: string]: nothing -> nothing {
    let srv = (server $CALLER)
    let res = (cli $CALLER (run-mark-argv $srv $run_id $FIELD $verdict))
    if $res.exit_code != 0 {
        print -e $"($CALLER): run mark failed for ($run_id) \(($verdict)\): ($res.stderr | str trim | str substring 0..240)"
        exit 1
    }
    print $"($CALLER): marked ($run_id) \($FIELD = ($verdict)\)"
}

def main [--smoke]: nothing -> nothing {
    let srv = (server $CALLER)
    let queue_res = (cli $CALLER (run-queue-argv $srv $FIELD $WORKFLOW))
    if $queue_res.exit_code != 0 {
        print -e $"($CALLER): run queue read failed: ($queue_res.stderr | str trim | str substring 0..240)"
        exit 1
    }
    let entries = (entries-from $queue_res.stdout)
    print $"($CALLER): worklist holds ($entries | length) run\(s\) on \($FIELD)/($WORKFLOW)\)"

    mut packets = []
    mut marked_enabled = []
    mut marked_refused = []
    mut skipped_parked = []
    for entry in $entries {
        let run_id = $entry.run.id
        let work_item = ($entry | get -o work_item | default '')
        let project = ($entry | get -o project | default '')
        let has_pr = ($entry.run | get -o pull_request | default null) != null

        if $entry.verdict == "parked" {
            # An open operator escalation re-enters every pass: the
            # agent pass re-engages with the fork's context.
            $skipped_parked = ($skipped_parked | append $run_id)
            $packets = ($packets | append {run_id: $run_id, work_item: (null-if-empty $work_item), verdict: "parked", painpoints: (distinct-painpoints (read-observations $run_id)), radius_items: (read-radius $project $work_item | length), has_pull_request: $has_pr})
            continue
        }

        let painpoints = (distinct-painpoints (read-observations $run_id))
        let radius_count = (read-radius $project $work_item | length)
        if ($painpoints | is-empty) and $radius_count == 0 {
            # The trivial mass: mark directly, no LLM call.
            if $has_pr {
                mark-run $run_id "enabled"
                $marked_enabled = ($marked_enabled | append $run_id)
            } else {
                mark-run $run_id "refused"
                $marked_refused = ($marked_refused | append $run_id)
            }
        } else {
            $packets = ($packets | append {run_id: $run_id, work_item: (null-if-empty $work_item), verdict: "pending", painpoints: $painpoints, radius_items: $radius_count, has_pull_request: $has_pr})
        }
    }

    print $"($CALLER): marked \(($marked_enabled | length)\) enabled, \(($marked_refused | length)\) refused; \(($packets | length)\) packet\(s\) for the agent pass"
    let adjudication = {
        packets: $packets,
        marked_enabled: $marked_enabled,
        marked_refused: $marked_refused,
        parked_reentered: $skipped_parked,
    }
    if $smoke {
        # The smoke drive: print the packet record for assertion, no
        # routing envelope (no engine consumes this output).
        $adjudication | to json --raw | print
        return
    }
    let label = (if ($packets | is-empty) { "Queue empty" } else { "Runs pending adjudication" })
    {preferred_next_label: $label, context_updates: {output.adjudication: $adjudication}} | to json --raw | print
}

# Pure: null for an empty work-item string, the string otherwise (the
# packet shape stays JSON-clean).
def null-if-empty [value: string]: nothing -> any {
    if ($value | is-empty) { null } else { $value }
}
