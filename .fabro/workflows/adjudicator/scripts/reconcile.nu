#!/usr/bin/env nu
# The adjudicator reconcile stage (denkhaus/work#1615): the LLM-free
# truth record after the agent pass. The 2026-11-22 incident: the agent
# pass marked a packet run merge=enabled six times (the verdict records
# were always correct) and then rewrote its prose journal into "no
# re-mark — already settled" — the operator and the next pass read the
# prose and misread a healthy pipeline as a stranded one. This stage
# makes the sweep's disposition mechanical, so the journal can never be
# the only record:
#
#   1. The fast path's `fact=packets-emitted runs=<ids>` observation
#      (its own run, recorded when packets are non-empty) is the
#      worklist contract — WHAT the agent pass was asked to settle.
#   2. Each packet run classifies from the queue doors alone:
#        closable queue under the merge=enabled requirement (the
#          merger's own worklist read)  -> enabled
#        merge queue, verdict=parked      -> parked
#        merge queue, verdict=pending     -> HOLE (the pass left a
#          packet unsettled; the soft exit keeps the queue for the
#          next pass — gap-safe, never a silent Reconciled)
#        neither queue                    -> refused (the refusal
#          frees the run from every queue)
#   3. ONE note observation records the classification verbatim — the
#      operator reads this, not the agent pass's prose.
#
# The stage exits 0 on holes too: the soft exit ROUTES, the run never
# fails on the pass's own unfinished work (the failed-sweep soft exit
# stays the graph's repair path). Fact-failure discipline unchanged: a
# failed door read or a missing packets note fails the stage (exit 1,
# the engine retries).

use ../../../scripts/lib/work_channel.nu [
    server, cli, observation-list-argv, observation-record-argv, read-queue-hard, emit-envelope
]

const CALLER = 'adjudicator-reconcile'
const WORKFLOW = 'develop'

# The packet ids from the lap's OWN observations: the LAST
# `fact=packets-emitted runs=<ids>` note (the fast path retries could
# record more than one; the last is the freshest sweep). Null when the
# note never landed — the fast path's contract breach, a fact failure.
def read-packet-note [own_run: string]: nothing -> any {
    let srv = (server $CALLER)
    let res = (cli $CALLER (observation-list-argv $srv $own_run))
    if $res.exit_code != 0 {
        print -e $"($CALLER): observation list failed for ($own_run): ($res.stderr | str trim | str substring 0..240)"
        exit 1
    }
    let parsed = (try { $res.stdout | from json } catch { null })
    let observations = (if ($parsed | describe | str starts-with "record") {
        $parsed | get -o observations | default []
    } else { [] })
    let notes = ($observations
        | where {|o| $o.category == "note"}
        | where {|o| ($o.text | str trim | str starts-with 'fact=packets-emitted runs=')})
    if ($notes | is-empty) { null } else { $notes | last | get text }
}

# The packet run ids off the note text: everything after
# `fact=packets-emitted runs=`, comma-separated, deduped, whitespace
# trimmed.
def packet-ids [note: string]: nothing -> list<string> {
    let raw = ($note | str trim | str replace 'fact=packets-emitted runs=' '')
    $raw | split row ',' | str trim | where {|id| not ($id | is-empty)} | uniq
}

def main [--smoke]: nothing -> nothing {
    let own_run = ($env.FABRO_RUN_ID? | default '')
    if ($own_run | is-empty) {
        print -e $"($CALLER): FABRO_RUN_ID is not bound — the stage reads its own lap's packets"
        exit 1
    }
    let note = (read-packet-note $own_run)
    if $note == null {
        print -e $"($CALLER): no fact=packets-emitted note on ($own_run) — the fast path must record the packet list before the agent pass runs"
        exit 1
    }
    let ids = (packet-ids $note)

    # The two queue reads: the merge field (pending = hole, parked =
    # parked) and the merger's worklist read (closable under the
    # merge=enabled requirement = enabled).
    let merge_entries = (read-queue-hard $CALLER 'merge' $WORKFLOW)
    let enabled_entries = (read-queue-hard $CALLER 'closable' $WORKFLOW '' 'merge=enabled')

    mut enabled = []
    mut parked = []
    mut refused = []
    mut holes = []
    for id in $ids {
        let merge_entry = ($merge_entries | where {|e| $e.run.id == $id} | get -o 0)
        let in_enabled = ($enabled_entries | where {|e| $e.run.id == $id} | is-not-empty)
        if $in_enabled {
            $enabled = ($enabled | append $id)
        } else if $merge_entry != null and $merge_entry.verdict == 'pending' {
            $holes = ($holes | append $id)
        } else if $merge_entry != null and $merge_entry.verdict == 'parked' {
            $parked = ($parked | append $id)
        } else {
            $refused = ($refused | append $id)
        }
    }

    # The truth record: one note observation, the machine-classified
    # disposition of every packet. HARD (a failed record fails the
    # stage — the idempotent retry re-writes it). Empty classes render
    # as `-` so the line is grep-stable.
    let spell = {|ids| if ($ids | is-empty) { '-' } else { $ids | str join ',' } }
    let truth = $"verdict: reconcile ($ids | length) packets — enabled (do $spell $enabled); parked (do $spell $parked); refused (do $spell $refused); holes (do $spell $holes)"
    let srv = (server $CALLER)
    let res = (cli $CALLER (observation-record-argv $srv 'note' $truth))
    if $res.exit_code != 0 {
        print -e $"($CALLER): reconcile record failed: ($res.stderr | str trim | str substring 0..240)"
        exit 1
    }
    print $"($CALLER): ($truth)"

    let label = (if ($holes | is-empty) { "Reconciled" } else { "Holes remain" })
    let reconcile_record = {
        packets: $ids,
        enabled: $enabled,
        parked: $parked,
        refused: $refused,
        holes: $holes,
        truth: $truth,
    }
    emit-envelope $label 'reconcile' $reconcile_record $smoke
}
