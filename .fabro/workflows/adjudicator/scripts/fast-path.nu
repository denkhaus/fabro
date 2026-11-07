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
# collapses), no ROUTED note classes, and an empty closure radius.
# ROUTED NOTES supersede stage-C decision 5 (denkhaus/work#1146, user
# decision, sprint 81): a painpoint still forces the agent pass, AND a
# note whose machine head names a routed class escalates too — the
# marker registry below is the deterministic trigger surface.
# Unclassified notes never trigger anything, but they RIDE the packet
# context: the agent pass reads them whenever it runs. The trivial
# verdict splits by pull request: WITH one is merge: enabled; WITHOUT
# one (a failed or empty run) is merge: refused — the verdict means
# content-mergeable, and nothing to merge means refused.
#
# Parked runs re-enter every pass (only enabled and refused free a run):
# they route to the agent pass again — an open escalation is a judgment
# the fast path cannot resolve.
#
# HUMAN_NEEDED (work#1072, stage-F consequence): a run whose terminal
# kind is human_needed parks DIRECTLY, LLM-free — the deadlock exit
# itself means a human decision pends; the park needs no judgment. The
# operator answers through the work-item comment channel; the next
# passes re-enter as parked and the agent pass verdicts once answered.
#
# FACT FAILURE DISCIPLINE (user decision 2026-11-01): a door call that
# fails FAILS THE STAGE — exit non-zero, the engine retries; never
# absorb a failed read or write.

use ../../../scripts/lib/fact_head.nu [parse-head, head-of]
use ../../../scripts/lib/work_channel.nu [
    server, cli, observation-list-argv, work-radius-argv, read-queue-hard, mark-run-hard, emit-envelope, title-own-run
]

const CALLER = 'adjudicator-fast-path'
const FIELD = 'merge'
const WORKFLOW = 'develop'

# The routed note classes (work#1146): note FACT names (the machine
# head's `fact=` token) that deterministically escalate a run to the
# agent pass. The develop start stage owns the lane-drained marker
# (work#1144); new classes join HERE and as a `fact=<name>` head in the
# recording stage, never in prose.
const ROUTED_NOTE_HEADS = ['lane-drained']

# Pure: the note texts whose machine head names a routed fact — the
# deterministic escalation surface beside the painpoints.
def routed-notes [observations: list<any>]: nothing -> list<string> {
    $observations
    | where {|o| $o.category == "note"}
    | where {|o|
        let head = (parse-head $o.text)
        $head != null and ($head | get -o fact | default '') in $ROUTED_NOTE_HEADS
    }
    | get text
}

# Pure: every note's text — the packet context the agent pass reads
# when it runs (unclassified notes ride along, they never trigger).
def note-texts [observations: list<any>]: nothing -> list<string> {
    $observations | where {|o| $o.category == "note"} | get text
}

# The agent-pass packet: one shape, built at both arms (the parked
# re-entry and the pending escalation differ only in verdict and
# precomputed inputs).
def packet [
    run_id: string,
    work_item: string,
    verdict: string,
    painpoints: list<string>,
    routed: list<string>,
    observations: list<any>,
    radius_items: int,
    has_pr: bool,
]: nothing -> record {
    {
        run_id: $run_id,
        work_item: (null-if-empty $work_item),
        verdict: $verdict,
        painpoints: $painpoints,
        routed_notes: $routed,
        notes: (note-texts $observations),
        radius_items: $radius_items,
        has_pull_request: $has_pr,
    }
}

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
# the fold, a re-sweep is gap-safe). The wrapper is the shared policy
# (#1132); this alias carries the script's FIELD.
def mark-run [run_id: string, verdict: string]: nothing -> nothing {
    mark-run-hard $CALLER $FIELD $run_id $verdict
}

def main [--smoke]: nothing -> nothing {
    let entries = (read-queue-hard $CALLER $FIELD $WORKFLOW)

    mut packets = []
    mut marked_enabled = []
    mut marked_refused = []
    mut parked_human = []
    mut skipped_parked = []
    for entry in $entries {
        let run_id = $entry.run.id
        let work_item = ($entry | get -o work_item | default '')
        let project = ($entry | get -o project | default '')
        let has_pr = ($entry.run | get -o pull_request | default null) != null

        if $entry.verdict == "parked" {
            # An open operator escalation re-enters every pass: the
            # agent pass re-engages with the fork's context.
            let observations = (read-observations $run_id)
            $skipped_parked = ($skipped_parked | append $run_id)
            $packets = ($packets | append (packet $run_id $work_item "parked" (distinct-painpoints $observations) (routed-notes $observations) $observations (read-radius $project $work_item | length) $has_pr))
            continue
        }

        let terminal_kind = ($entry.run | get -o lifecycle | get -o status | get -o kind | default '')
        if $terminal_kind == "human_needed" {
            # work#1072 (stage-F design consequence, grilled 2026-11-05):
            # the run itself asks for a human — the deadlock exit means a
            # decision pends. Park directly, LLM-free: the park needs no
            # judgment; the operator answers through the work-item
            # comment channel; the next passes re-enter (parked stays
            # queued) and the agent pass verdicts once answered.
            mark-run $run_id "parked"
            $parked_human = ($parked_human | append $run_id)
            continue
        }

        let observations = (read-observations $run_id)
        let painpoints = (distinct-painpoints $observations)
        let routed = (routed-notes $observations)
        let radius_count = (read-radius $project $work_item | length)
        if ($painpoints | is-empty) and ($routed | is-empty) and $radius_count == 0 {
            # The trivial mass: mark directly, no LLM call.
            if $has_pr {
                mark-run $run_id "enabled"
                $marked_enabled = ($marked_enabled | append $run_id)
            } else {
                mark-run $run_id "refused"
                $marked_refused = ($marked_refused | append $run_id)
            }
        } else {
            $packets = ($packets | append (packet $run_id $work_item "pending" $painpoints $routed $observations $radius_count $has_pr))
        }
    }

    # The pass title (#1159 follow-through): the sweep shape lands on
    # the board tile — the operator reads what THIS pass did without
    # opening the run. Advisory (work_channel::title-own-run).
    title-own-run $CALLER $"Adjudicator: marked \(($marked_enabled | length)\) enabled, \(($marked_refused | length)\) refused; \(($packets | length)\) packet\(s\) for the agent pass"
    print $"($CALLER): marked \(($marked_enabled | length)\) enabled, \(($marked_refused | length)\) refused; \(($packets | length)\) packet\(s\) for the agent pass"
    let adjudication = {
        packets: $packets,
        marked_enabled: $marked_enabled,
        marked_refused: $marked_refused,
        parked_human_needed: $parked_human,
        parked_reentered: $skipped_parked,
    }
    let label = (if ($packets | is-empty) { "Queue empty" } else { "Runs pending adjudication" })
    emit-envelope $label 'adjudication' $adjudication $smoke
}

# Pure: null for an empty work-item string, the string otherwise (the
# packet shape stays JSON-clean).
def null-if-empty [value: string]: nothing -> any {
    if ($value | is-empty) { null } else { $value }
}
