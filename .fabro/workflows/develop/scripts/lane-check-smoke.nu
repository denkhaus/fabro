#!/usr/bin/env nu
# Smoke test for the develop lane check (denkhaus/work#1144): drives
# lane-check.nu end to end through the channel shim — an empty lane
# records the lane-drained NOTE (the routed marker, work#1146) and
# routes Lane drained; an open item routes Lane open with NO record;
# an item in ANOTHER lane counts as drained (the lane filter is the
# shim's fidelity arm); a missing server refuses loudly.

const SCRIPTS_DIR = (path self | path dirname)

use ../../../scripts/lib/smoke_harness.nu [
    start-fail-log, fail, end-fail-gate, write-shim-state, drive, refuses-missing-server
]
load-env {SMOKE_FAIL_LOG: (start-fail-log 'develop-lane-check-smoke')}

const DEVELOP_RUN = "01LANECHECKRUN0000000000000A"

def envelope-label [result: any]: nothing -> string {
    let envelope = (try { $result.stdout | lines | compact | where {|l| $l | str starts-with '{'} | last | from json } catch { null })
    if $envelope == null {
        fail $"the check must print its routing envelope JSON: ($result.stdout | str substring 0..300)"
    }
    $envelope | get -o preferred_next_label | default ''
}

def recorded-notes [state: string]: nothing -> list<any> {
    open --raw $state | from json | get -o observations | default []
}

# --- Arm 1: the drained lane — the routed marker records, the run ends
# PR-less through the Lane drained label.
let drained_state = (mktemp -t lane-check-drained.XXXXXX.json)
write-shim-state $drained_state {items: [], claims: [], nextKey: 9000, log: [], runs: [], marks: [], observations: [], radius: {}}
let drained = (drive ($SCRIPTS_DIR | path join "lane-check.nu") $drained_state ['--project', 'fabro-proof', '--lane', 'develop'] $DEVELOP_RUN)
if $drained.exit_code != 0 {
    fail $"the drained drive must exit 0: ($drained.stderr | str substring 0..300)"
}
if (envelope-label $drained) != 'Lane drained' {
    fail $"an empty lane must route Lane drained: ($drained.stdout | str substring 0..240)"
}
let notes = (recorded-notes $drained_state)
if ($notes | length) != 1 {
    fail $"the drained lane records exactly one note: ($notes | to json -r)"
}
if $notes.0.category != 'note' {
    fail $"the drained record is a NOTE — notes ride the routing, painpoints force: ($notes.0 | to json -r)"
}
if not ($notes.0.text | str starts-with 'fact=lane-drained lane=develop') {
    fail $"the note head carries the routed marker: ($notes.0 | to json -r)"
}

# --- Arm 2: the open lane — no record, straight to the planner.
let open_state = (mktemp -t lane-check-open.XXXXXX.json)
write-shim-state $open_state {
    items: [
        {itemKey: "denkhaus/work#9", title: "the claimable item", body: "the item body", state: "open", labels: ["lane:develop"]}
    ],
    claims: [], nextKey: 9010, log: [], runs: [], marks: [], observations: [], radius: {}
}
let open = (drive ($SCRIPTS_DIR | path join "lane-check.nu") $open_state ['--project', 'fabro-proof', '--lane', 'develop'] $DEVELOP_RUN)
if $open.exit_code != 0 {
    fail $"the open drive must exit 0: ($open.stderr | str substring 0..300)"
}
if (envelope-label $open) != 'Lane open' {
    fail $"an open lane must route Lane open: ($open.stdout | str substring 0..240)"
}
if ((recorded-notes $open_state) | length) != 0 {
    fail "an open lane records nothing — the planner owns the claim"
}

# --- Arm 3: an item in ANOTHER lane is not claimable — the lane label
# filters (the shim's lane fidelity arm).
let foreign_state = (mktemp -t lane-check-foreign.XXXXXX.json)
write-shim-state $foreign_state {
    items: [
        {itemKey: "denkhaus/work#8", title: "the loop lane item", body: "the item body", state: "open", labels: ["lane:loop"]}
    ],
    claims: [], nextKey: 9020, log: [], runs: [], marks: [], observations: [], radius: {}
}
let foreign = (drive ($SCRIPTS_DIR | path join "lane-check.nu") $foreign_state ['--project', 'fabro-proof', '--lane', 'develop'] $DEVELOP_RUN)
if $foreign.exit_code != 0 {
    fail $"the foreign-lane drive must exit 0: ($foreign.stderr | str substring 0..300)"
}
if (envelope-label $foreign) != 'Lane drained' {
    fail $"another lane's item must not hold the develop line open: ($foreign.stdout | str substring 0..240)"
}

# --- Negative arm: a missing FABRO_SERVER is refused loudly.
refuses-missing-server ($SCRIPTS_DIR | path join "lane-check.nu") $drained_state

end-fail-gate
print "develop-lane-check-smoke: ok — drained records the routed marker, open hands to the planner, foreign lanes do not hold the line open"
