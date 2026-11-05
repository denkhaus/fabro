#!/usr/bin/env nu
# Smoke test for the merger fast path (ADR-0039 stage E,
# denkhaus/work#1107): drives fast-path.nu end to end through the
# channel shim — the clean mass merges through the door and marks
# closable=enabled, a conflicted merge routes to the agent pass as a
# packet (the merge_conflict signal), a malformed PR-less enabled run
# parks, and parked runs re-enter as packets. The smoke machine is
# SKIPPED in this drive (cargo check has no place in a unit smoke: the
# --smoke flag short-circuits before the tree work, and the smoke
# helper's transport arms are pinned by the fixture's conflicted run
# instead). No live server: the shim fakes every door; the argv grammar
# against the REAL parser is pinned below (the closeout-smoke
# precedent).

const SCRIPTS_DIR = (path self | path dirname)

use ../../../scripts/lib/smoke_harness.nu [
    start-fail-log, fail, end-fail-gate, write-shim-state, drive, refuses-missing-server, pin-argvs
]
load-env {SMOKE_FAIL_LOG: (start-fail-log 'merger-fast-path-smoke')}

# The fixture worklist: three merge-enabled develop runs (clean,
# conflicted, PR-less malformed), one parked re-entry, one smoke
# workflow's run (the filter keeps it out), and the work#1126 arms —
# one merge-pending run (never adjudicated) and one merge-refused run
# (content rejected): the REQUIREMENT must keep both out of the merger
# worklist entirely.
const MERGER_RUN = "01MERGER00000000000000000A"
let state = (mktemp -t merger-fast-path.XXXXXX.json)
write-shim-state $state {
    items: [
        {itemKey: "denkhaus/work#1", title: "The clean landing", body: "the item body", state: "open", labels: []}
        {itemKey: "denkhaus/work#2", title: "The conflicted landing", body: "the item body", state: "open", labels: []}
    ],
    claims: [],
    nextKey: 9001,
    log: [],
    runs: [
        {runId: "01CLEANRUN0000000000000000A", workflowSlug: "develop", status: "succeeded", title: "clean merge run", pullRequest: {owner: "denkhaus", repo: "fabro", number: 601}, claimedItem: "denkhaus/work#1", project: "fabro"}
        {runId: "01CONFLICTRUN00000000000000B", workflowSlug: "develop", status: "succeeded", title: "the conflicted run", pullRequest: {owner: "denkhaus", repo: "fabro", number: 602}, claimedItem: "denkhaus/work#2", project: "fabro", conflicted: true}
        {runId: "01PRLESSRUN0000000000000000C", workflowSlug: "develop", status: "failed", title: "malformed enabled run", claimedItem: null, project: null}
        {runId: "01PARKEDMERGE00000000000000D", workflowSlug: "develop", status: "succeeded", title: "the parked escalation", pullRequest: {owner: "denkhaus", repo: "fabro", number: 603}, claimedItem: null, project: null}
        {runId: "01SMOKERUN0000000000000000E", workflowSlug: "smoke", status: "succeeded", title: "another workflow's run", claimedItem: null, project: null}
        {runId: "01PENDINGRUN000000000000000F", workflowSlug: "develop", status: "succeeded", title: "the never-adjudicated run", pullRequest: {owner: "denkhaus", repo: "fabro", number: 604}, claimedItem: null, project: null}
        {runId: "01REFUSEDRUN0000000000000010", workflowSlug: "develop", status: "failed", title: "the content-refused run", claimedItem: null, project: null}
    ],
    marks: [
        # The adjudicator enabled every worklist run's merge (the
        # closable queue's premise); one carries a parked CLOSABLE from
        # an earlier pass. The pending run carries NO merge mark; the
        # refused run's merge verdict is refused — the work#1126 arms.
        {runId: "01CLEANRUN0000000000000000A", field: "merge", verdict: "enabled", byRun: "01PREVPASS0000000000000000"}
        {runId: "01CONFLICTRUN00000000000000B", field: "merge", verdict: "enabled", byRun: "01PREVPASS0000000000000000"}
        {runId: "01PRLESSRUN0000000000000000C", field: "merge", verdict: "enabled", byRun: "01PREVPASS0000000000000000"}
        {runId: "01PARKEDMERGE00000000000000D", field: "merge", verdict: "enabled", byRun: "01PREVPASS0000000000000000"}
        {runId: "01REFUSEDRUN0000000000000010", field: "merge", verdict: "refused", byRun: "01PREVPASS0000000000000000"}
        {runId: "01PARKEDMERGE00000000000000D", field: "closable", verdict: "parked", byRun: "01PREVMERGER00000000000000F"}
    ],
    observations: [],
    radius: {},
    merges: [],
}

# --- The sweep drive (engine-mode argv; the routing envelope must land)
let sweep = (drive ($SCRIPTS_DIR | path join "fast-path.nu") $state [--smoke] $MERGER_RUN)
if $sweep.exit_code != 0 {
    fail $"the sweep must exit 0: ($sweep.stderr | str substring 0..300)"
}

# The clean run merged through the door and marked closable=enabled.
let after = (open --raw $state | from json)
let merges = ($after | get -o merges | default [])
if ("01CLEANRUN0000000000000000A" in ($merges | get -o runId | default [])) == false {
    fail $"the clean run must merge through the door: ($merges | to json -r)"
}
let closable_marks = ($after.marks | where {|m| $m.field == "closable" and $m.byRun == $MERGER_RUN})
let enabled = ($closable_marks | where {|m| $m.verdict == "enabled"})
if ($enabled | length) != 1 or $enabled.0.runId != "01CLEANRUN0000000000000000A" {
    fail $"the clean run must be the only enabled mark: ($closable_marks | to json -r)"
}

# The conflicted run: NO merge, NO mark — a packet for the agent pass.
if ("01CONFLICTRUN00000000000000B" in ($merges | get -o runId | default [])) {
    fail "the conflicted run must not merge through the fast path"
}
if (($closable_marks | where {|m| $m.runId == "01CONFLICTRUN00000000000000B"}) | length) != 0 {
    fail "the conflicted run must carry no fast-path verdict"
}
let integration = (try { $sweep.stdout | lines | compact | where {|l| $l | str starts-with '{'} | last | from json } catch { null })
if $integration == null {
    fail $"the smoke drive must print its integration record: ($sweep.stdout | str substring 0..300)"
}
let packets = ($integration | get -o packets | default [])
let packet_ids = ($packets | get -o run_id | default [])
if ("01CONFLICTRUN00000000000000B" not-in $packet_ids) {
    fail $"the conflicted run must packet for the agent pass: ($packets | to json -r)"
}
if ("01PARKEDMERGE00000000000000D" not-in $packet_ids) {
    fail $"the parked run must re-enter as a packet: ($packets | to json -r)"
}

# The malformed PR-less run parks (the operator escalation), never
# merges.
let malformed = ($closable_marks | where {|m| $m.runId == "01PRLESSRUN0000000000000000C" and $m.verdict == "parked"})
if ($malformed | length) != 1 {
    fail $"the PR-less enabled run must park: ($closable_marks | to json -r)"
}
if ("01PRLESSRUN0000000000000000C" in ($merges | get -o runId | default [])) {
    fail "the PR-less run must never reach the merge door"
}
# The workflow filter kept the smoke run out.
if ("01SMOKERUN0000000000000000E" in ($merges | get -o runId | default [])) {
    fail "the smoke workflow's run leaked through the filter"
}

# The work#1126 arms: the merge-pending and merge-refused runs never
# reach the merge door, never carry a verdict, never packet — the
# requirement keeps them out of the worklist entirely.
for excluded in ["01PENDINGRUN000000000000000F" "01REFUSEDRUN0000000000000010"] {
    if ($excluded in ($merges | get -o runId | default [])) {
        fail $"($excluded) must never reach the merge door — the requirement excludes it"
    }
    if (($closable_marks | where {|m| $m.runId == $excluded}) | length) != 0 {
        fail $"($excluded) must carry no fast-path verdict — the requirement excludes it"
    }
    if ($excluded in $packet_ids) {
        fail $"($excluded) must not packet for the agent pass — the requirement excludes it"
    }
}

# --- Negative arm: a missing FABRO_SERVER is refused loudly.
refuses-missing-server ($SCRIPTS_DIR | path join "fast-path.nu") $state

# --- The real-parser argv pin (the closeout-smoke precedent): the
# merge facade's grammar is verb-first, and the queue's requirement
# filter must parse against the REAL binary on a dead server.
do {
    let dead = "http://127.0.0.1:1"
    pin-argvs $dead [
        [run queue --field closable --requires merge=enabled --workflow develop --json --server $dead]
        [run merge "01M5844J1FDD" --server $dead]
    ]
}

rm -f $state
end-fail-gate
print "merger-fast-path-smoke: ok — clean merge + enabled mark, conflict packet, malformed park, parked re-entry, filter, argv grammar"

# Sourcing fast-path.nu imports its `def main`; nu auto-invokes it after
# the top level runs — exit explicitly so the smoke never reaches it.
exit 0
