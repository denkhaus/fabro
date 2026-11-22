#!/usr/bin/env nu
# Smoke test for the adjudicator reconcile stage (denkhaus/work#1615):
# the LLM-free truth record after the agent pass. The fast path's
# packets-emitted note (the machine-head observation the fast path
# records on its own run) is the worklist contract; the stage
# classifies every packet run from the queue doors — enabled (the
# closable queue under the merge=enabled requirement, the merger's own
# worklist read), parked (the merge queue's parked verdict), refused
# (present in neither) — records ONE authoritative observation, and
# routes. A packet run still pending in the merge queue is the HOLE
# (the pass declared work it never settled): the stage names it and
# routes the soft exit so the next pass re-enters cleanly. No live
# server: the shim fakes every door. The argv grammar against the REAL
# parser is pinned by the merger and closing-pass smokes (the same
# builders, already registered).

const SCRIPTS_DIR = (path self | path dirname)

use ../../../scripts/lib/smoke_harness.nu [
    start-fail-log, fail, end-fail-gate, write-shim-state, drive
]
load-env {SMOKE_FAIL_LOG: (start-fail-log 'adjudicator-reconcile-smoke')}

const ADJUDICATOR_RUN = "01ADJUDICATOR00000000000000A"

# The packet ids the fast path's note names (the machine-head text
# below is the same string the fast path records).
const PACKETS = "01PKTENABLED00000000000000A,01PKTPARKED00000000000000B,01PKTREFUSED0000000000000C"
const PACKETS_WITH_HOLE = "01PKTENABLED00000000000000A,01PKTHOLERUN0000000000000D"
const EMITTED_NOTE = $"fact=packets-emitted runs=($PACKETS)"
const EMITTED_NOTE_HOLE = $"fact=packets-emitted runs=($PACKETS_WITH_HOLE)"

# The settled fixture: one run per classification. ENABLED carries the
# merge verdict the agent pass wrote (the closable queue under the
# merge=enabled requirement lists it — the merger's worklist). PARKED
# stays queued on the merge field. REFUSED freed its run (present in
# neither queue). No pending run: everything the fast path escalated
# is settled.
let state = (mktemp -t adjudicator-reconcile.XXXXXX.json)
write-shim-state $state {
    items: [],
    claims: [],
    nextKey: 9001,
    log: [],
    runs: [
        {runId: "01PKTENABLED00000000000000A", workflowSlug: "develop", status: "succeeded", title: "the enabled packet", claimedItem: null, project: null}
        {runId: "01PKTPARKED00000000000000B", workflowSlug: "develop", status: "succeeded", title: "the parked packet", claimedItem: null, project: null}
        {runId: "01PKTREFUSED0000000000000C", workflowSlug: "develop", status: "failed", title: "the refused packet", claimedItem: null, project: null}
    ],
    marks: [
        {runId: "01PKTENABLED00000000000000A", field: "merge", verdict: "enabled", byRun: $ADJUDICATOR_RUN}
        {runId: "01PKTPARKED00000000000000B", field: "merge", verdict: "parked", byRun: $ADJUDICATOR_RUN}
        {runId: "01PKTREFUSED0000000000000C", field: "merge", verdict: "refused", byRun: $ADJUDICATOR_RUN}
    ],
    observations: [
        {runId: $ADJUDICATOR_RUN, seq: 1, category: "note", text: $EMITTED_NOTE}
    ],
    radius: {}
}

let settled = (drive ($SCRIPTS_DIR | path join "reconcile.nu") $state [] $ADJUDICATOR_RUN)
if $settled.exit_code != 0 {
    fail $"the reconcile must exit 0 on a settled sweep: ($settled.stderr | str substring 0..300)"
}
if not ($settled.stdout | str contains '"Reconciled"') {
    fail $"a fully settled sweep must route Reconciled: ($settled.stdout | str substring 0..300)"
}

# The truth record: ONE note observation on the lap's own run, the
# machine-classified disposition of EVERY packet — the operator and the
# next pass read this, never the agent pass's prose.
let after = (open --raw $state | from json)
let notes = ($after.observations
    | where {|o| $o.runId == $ADJUDICATOR_RUN and $o.category == "note"}
    | where {|o| $o.text =~ '^verdict: reconcile'})
if ($notes | length) != 1 {
    fail $"exactly one reconcile truth note expected: ($notes | to json -r)"
}
let truth = $notes.0.text
if not ($truth =~ 'enabled 01PKTENABLED00000000000000A') {
    fail $"the enabled packet must classify from the closable queue: ($truth)"
}
if not ($truth =~ 'parked 01PKTPARKED00000000000000B') {
    fail $"the parked packet must classify from the merge queue: ($truth)"
}
if not ($truth =~ 'refused 01PKTREFUSED0000000000000C') {
    fail $"the freed run must classify as refused: ($truth)"
}
if not ($truth =~ 'holes -') {
    fail $"a settled sweep must name no holes: ($truth)"
}

# The hole fixture: the agent pass left one packet run pending — no
# merge verdict on it, so it still sits in the merge queue. The stage
# must NAME the hole and route the soft exit (the queue stays, the
# next pass re-enters) — never a silent "Reconciled".
let state_hole = (mktemp -t adjudicator-reconcile-hole.XXXXXX.json)
write-shim-state $state_hole {
    items: [],
    claims: [],
    nextKey: 9001,
    log: [],
    runs: [
        {runId: "01PKTENABLED00000000000000A", workflowSlug: "develop", status: "succeeded", title: "the enabled packet", claimedItem: null, project: null}
        {runId: "01PKTHOLERUN0000000000000D", workflowSlug: "develop", status: "succeeded", title: "the hole", claimedItem: null, project: null}
    ],
    marks: [
        {runId: "01PKTENABLED00000000000000A", field: "merge", verdict: "enabled", byRun: $ADJUDICATOR_RUN}
    ],
    observations: [
        {runId: $ADJUDICATOR_RUN, seq: 1, category: "note", text: $EMITTED_NOTE_HOLE}
    ],
    radius: {}
}

let holed = (drive ($SCRIPTS_DIR | path join "reconcile.nu") $state_hole [] $ADJUDICATOR_RUN)
if $holed.exit_code != 0 {
    fail $"the reconcile exits 0 on holes too — the soft exit routes, the stage never fails the run: ($holed.stderr | str substring 0..300)"
}
if not ($holed.stdout | str contains '"Holes remain"') {
    fail $"a pending packet must route the soft exit: ($holed.stdout | str substring 0..300)"
}
let after_hole = (open --raw $state_hole | from json)
let hole_notes = ($after_hole.observations
    | where {|o| $o.runId == $ADJUDICATOR_RUN and $o.category == "note"}
    | where {|o| $o.text =~ '^verdict: reconcile'})
if ($hole_notes | length) != 1 {
    fail $"exactly one reconcile truth note expected: ($hole_notes | to json -r)"
}
if not ($hole_notes.0.text =~ 'holes 01PKTHOLERUN0000000000000D') {
    fail $"the hole must be named in the truth record: ($hole_notes.0.text)"
}

print "adjudicator-reconcile-smoke: ok — settled classification (enabled/parked/refused), hole named + soft exit, one truth note per pass"
end-fail-gate
