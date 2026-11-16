#!/usr/bin/env nu
# The triager sweep (ADR-0044, denkhaus/work#1365): the LLM-free batch
# read over the global observation feed. One command stage walks the
# untriaged painpoints — oldest first (the backlog ages out ahead of the
# fresh), deduped on the fact head (engine retries re-record the first
# fact; the consumer collapses), capped at the batch size — and emits
# them as packets for the judgment pass (the evidence pipe: scripts
# fetch, the agent judges).
#
# The feed is the WORKLIST (the ADR-0044 separation): workflow-agnostic
# and state-driven, never a verdict queue — a settled run's painpoints
# stay reachable, and a future painpoint-producing workflow needs no
# verdict semantics to be seen.
#
# FACT FAILURE DISCIPLINE (the standing rule): a door call that fails
# FAILS THE STAGE — exit non-zero, the engine retries; never absorb a
# failed read.

use ../../../scripts/lib/fact_head.nu [parse-head, head-of]
use ../../../scripts/lib/work_channel.nu [
    server, cli, observation-feed-argv, emit-envelope, title-own-run
]

const CALLER = 'triager-sweep'
# One pass works at most this many painpoints: the agent pass judges
# serially, and a backlog bigger than this is a drain the next ticks
# continue (the feed is the worklist; nothing is lost).
const BATCH = 20

# Pure: the DISTINCT painpoints of the feed page — dedupe on the machine
# head (line one): an engine retry of a recording stage re-records the
# first fact, and the consumer collapses (the grilled keyed form, the
# adjudicator fast path's rule verbatim).
def distinct-painpoints [observations: list<any>]: nothing -> list<any> {
    mut seen = []
    mut kept = []
    for observation in $observations {
        let head = (parse-head $observation.text)
        let key = (if $head != null { head-of $head } else { $observation.text })
        if not ($key in $seen) {
            $seen = ($seen | append $key)
            $kept = ($kept | append $observation)
        }
    }
    $kept
}

def main [--smoke]: nothing -> nothing {
    let srv = (server $CALLER)
    let res = (cli $CALLER (observation-feed-argv $srv --category 'painpoint' --triage 'untriaged' --sort-by 'recorded' --limit $BATCH --page 1))
    if $res.exit_code != 0 {
        print -e $"($CALLER): observation feed read failed: ($res.stderr | str trim | str substring 0..240)"
        exit 1
    }
    let parsed = (try { $res.stdout | from json } catch { null })
    if not ($parsed | describe | str starts-with "record") {
        print -e $"($CALLER): observation feed stdout is not the GlobalObservationList record"
        exit 1
    }
    let observations = ($parsed | get -o observations | default [])
    let packets = (distinct-painpoints $observations | each {|observation|
        {
            run_id: $observation.run_id,
            seq: $observation.seq,
            recorded_at: $observation.recorded_at,
            text: $observation.text,
        }
    })

    # The pass title (#1159 follow-through): the sweep shape lands on
    # the board tile — the operator reads what THIS pass works without
    # opening the run.
    title-own-run $CALLER $"Triager: \(($packets | length)) untriaged painpoint\(s\) this pass"
    print $"($CALLER): \(($packets | length)) untriaged painpoint\(s\) this pass"
    let label = (if ($packets | is-empty) { "Feed empty" } else { "Painpoints pending triage" })
    emit-envelope $label 'triage' {packets: $packets} $smoke
}
