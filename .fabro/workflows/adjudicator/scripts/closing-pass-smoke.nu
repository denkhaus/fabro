#!/usr/bin/env nu
# Smoke test for the adjudicator closing pass (ADR-0039 stage E,
# denkhaus/work#1107): drives closing-pass.nu end to end through the
# channel shim — the verdict-flipped queue read (closable = enabled)
# lists exactly the merged runs, the empty-radius item CLOSES (the
# claim lands server-side), the open-radius item waits for the next
# pass, and an already-closed item's refusal is DONE, not a failure
# (the door's idempotence). No live server: the shim fakes every door;
# the argv grammar against the REAL parser is pinned below.

const SCRIPTS_DIR = (path self | path dirname)

use ../../../scripts/lib/smoke_harness.nu [
    start-fail-log, fail, end-fail-gate, write-shim-state, drive, refuses-missing-server, pin-argvs
]
load-env {SMOKE_FAIL_LOG: (start-fail-log 'adjudicator-closing-pass-smoke')}

# The fixture worklist: three closable-enabled develop runs — an
# empty-radius landing, an open-radius waiter, an already-closed re-entry
# — plus a pending run (the verdict filter keeps it out) and a smoke
# workflow's run (the workflow filter keeps it out).
const ADJUDICATOR_RUN = "01ADJUDICATOR00000000000000A"
let state = (mktemp -t closing-pass.XXXXXX.json)
write-shim-state $state {
    items: [
        {itemKey: "denkhaus/work#1", title: "The clean landing", body: "the item body", state: "open", labels: []}
        {itemKey: "denkhaus/work#2", title: "The radius waiter", body: "the item body", state: "open", labels: []}
        {itemKey: "denkhaus/work#3", title: "Already landed", body: "the item body", state: "closed", labels: []}
    ],
    claims: [],
    nextKey: 9001,
    log: [],
    runs: [
        {runId: "01LANDINGRUN0000000000000A", workflowSlug: "develop", status: "succeeded", title: "the clean landing run", pullRequest: {owner: "denkhaus", repo: "fabro", number: 601}, claimedItem: "denkhaus/work#1", project: "fabro"}
        {runId: "01RADIUSRUN00000000000000B", workflowSlug: "develop", status: "succeeded", title: "the radius waiter run", pullRequest: {owner: "denkhaus", repo: "fabro", number: 602}, claimedItem: "denkhaus/work#2", project: "fabro"}
        {runId: "01CLOSEDRUN00000000000000C", workflowSlug: "develop", status: "succeeded", title: "the already-landed re-entry", pullRequest: {owner: "denkhaus", repo: "fabro", number: 603}, claimedItem: "denkhaus/work#3", project: "fabro"}
        {runId: "01PENDINGRUN0000000000000D", workflowSlug: "develop", status: "succeeded", title: "not yet merged", claimedItem: null, project: null}
        {runId: "01SMOKERUN00000000000000E", workflowSlug: "smoke", status: "succeeded", title: "another workflow's run", claimedItem: null, project: null}
    ],
    marks: [
        {runId: "01LANDINGRUN0000000000000A", field: "closable", verdict: "enabled", byRun: "01PREVMERGER00000000000000F"}
        {runId: "01RADIUSRUN00000000000000B", field: "closable", verdict: "enabled", byRun: "01PREVMERGER00000000000000F"}
        {runId: "01CLOSEDRUN00000000000000C", field: "closable", verdict: "enabled", byRun: "01PREVMERGER00000000000000F"}
        {runId: "01PENDINGRUN0000000000000D", field: "closable", verdict: "pending", byRun: "01PREVMERGER00000000000000F"}
    ],
    observations: [],
    radius: {
        "denkhaus/work#2": [{itemKey: "denkhaus/work#4", title: "The open blocker", state: "open"}]
    },
    merges: [],
}

# --- The closing drive (engine-mode argv; the harness drive carries
# the shim env and the run id).
let closing = (drive ($SCRIPTS_DIR | path join "closing-pass.nu") $state [] $ADJUDICATOR_RUN)
if $closing.exit_code != 0 {
    fail $"the closing pass must exit 0: ($closing.stderr | str substring 0..300)"
}
if not ($closing.stdout | str contains '"Closing pass done"') {
    fail $"the pass must route Closing pass done: ($closing.stdout | str substring 0..300)"
}

# The landed item closed; the waiter stayed open; the closed item's
# refusal counted as already-closed.
let after = (open --raw $state | from json)
let item = {|key| $after.items | where {|it| $it.itemKey == $key} | get -o 0 }
if (do $item "denkhaus/work#1").state != "closed" {
    fail $"the empty-radius item must land: ($after.items | to json -r)"
}
if (do $item "denkhaus/work#2").state != "open" {
    fail $"the open-radius item must wait for the next pass: ($after.items | to json -r)"
}
# The close calls logged with the ENTRY's project (never a hardcode —
# work#1113's contract extends to the closing pass).
let close_log = ($after | get -o log | default [] | where {|l| ($l.argv | str join ' ') =~ 'close'})
if ($close_log | length) != 3 {
    fail $"every closable-enabled claimed item gets exactly one close call: ($close_log | to json -r)"
}
if (($close_log | where {|l| $l.flags?.project? != "fabro"} | length) != 0) {
    fail $"the close must carry the entry's project: ($close_log | to json -r)"
}
# The pending run and the smoke run never reached the close door.
if (($close_log | where {|l| ($l.argv | str join ' ') =~ 'work#2'}) | length) != 1 {
    fail "the radius waiter's close call happened (and was refused by the radius)"
}

# --- Negative arm: a missing FABRO_SERVER is refused loudly.
refuses-missing-server ($SCRIPTS_DIR | path join "closing-pass.nu") $state

# --- The real-parser argv pin: the verdict-flipped queue read and the
# work close grammar against the REAL binary on a dead server.
let dead = "http://127.0.0.1:1"
pin-argvs $dead [
    [run queue --field closable --verdict enabled --workflow develop --json --server $dead]
    [work --server $dead close --project fabro "denkhaus/work#1" --reason "landed via merger"]
]

rm -f $state
end-fail-gate
print "adjudicator-closing-pass-smoke: ok — verdict-flipped read, empty-radius lands, open-radius waits, closed item is done, entry project, argv grammar"

# Sourcing closing-pass.nu imports its `def main`; nu auto-invokes it
# after the top level runs — exit explicitly so the smoke never reaches
# it.
exit 0
