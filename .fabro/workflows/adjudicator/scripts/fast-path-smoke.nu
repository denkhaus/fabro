#!/usr/bin/env nu
# Smoke test for the adjudicator fast path (ADR-0039 stage D part 2,
# denkhaus/work#1107): drives fast-path.nu end to end through the
# channel shim — the trivial mass marks directly (split by pull
# request), non-trivial runs route to the agent pass as packets
# (painpoints, radius, parked re-entry), duplicate fact heads collapse
# (the engine-retry dedupe), and the routing envelope matches the
# engine's command-stage contract. No live server: the shim fakes every
# door; the argv grammar against the REAL parser is pinned separately
# below (the closeout-smoke precedent).

const SCRIPTS_DIR = (path self | path dirname)

use ../../../scripts/lib/smoke_harness.nu [
    start-fail-log, fail, end-fail-gate, shim-env, write-shim-state, pin-argv
]
load-env {SMOKE_FAIL_LOG: (start-fail-log 'adjudicator-fast-path-smoke')}

const SHIM = ($SCRIPTS_DIR | path join '..' '..' '..' 'scripts' 'fabro-work-shim.nu')

# The fixture worklist: four develop runs plus a smoke run (the filter
# keeps it out), one live run (never queues).
const ADJUDICATOR_RUN = "01ADJUDICATOR00000000000000A"
let state = (mktemp -t adjudicator-fast-path.XXXXXX.json)
write-shim-state $state {
    items: [
        {itemKey: "denkhaus/work#1", title: "Make the frobnicator queue bounded", body: "the item body", state: "open", labels: []}
        {itemKey: "denkhaus/work#2", title: "Clean the residual cache key", body: "the item body", state: "open", labels: []}
        {itemKey: "denkhaus/work#3", title: "The radius blocker", body: "the item body", state: "open", labels: []}
    ],
    claims: [],
    nextKey: 9001,
    log: [],
    runs: [
        {runId: "01TRIVIALPR00000000000000000", workflowSlug: "develop", status: "succeeded", title: "clean trivial run", pullRequest: {owner: "denkhaus", repo: "fabro", number: 501}, claimedItem: "denkhaus/work#1", project: "fabro"}
        {runId: "01TRIVIALNOPR0000000000000000", workflowSlug: "develop", status: "failed", title: "failed without a pull request", claimedItem: null, project: null}
        {runId: "01PAINPoinTRUN000000000000000", workflowSlug: "develop", status: "succeeded", title: "run with painpoints", pullRequest: {owner: "denkhaus", repo: "fabro", number: 502}, claimedItem: "denkhaus/work#2", project: "fabro-test"}
        {runId: "01PARKEDRUN00000000000000000", workflowSlug: "develop", status: "succeeded", title: "the parked escalation", pullRequest: null, claimedItem: "denkhaus/work#2", project: "fabro"}
        {runId: "01HUMANNEEDED00000000000000A", workflowSlug: "develop", status: "human_needed", title: "the deadlock ask", pullRequest: {owner: "denkhaus", repo: "fabro", number: 503}, claimedItem: "denkhaus/work#3", project: "fabro"}
        {runId: "01SMOKERUN000000000000000000", workflowSlug: "smoke", status: "succeeded", title: "another workflow's run", claimedItem: null, project: null}
        {runId: "01LIVERUN00000000000000000000", workflowSlug: "develop", status: "running", title: "still going", claimedItem: null, project: null}
    ],
    marks: [
        {runId: "01PARKEDRUN00000000000000000", field: "merge", verdict: "parked", byRun: "01PREVPASS0000000000000000"}
    ],
    observations: [
        # 01TRIVIALPR: one NOTE (visible patch) — notes never trigger.
        {runId: "01TRIVIALPR00000000000000000", seq: 1, category: "note", text: "fact=patch-visibility verdict=visible base=abc1234 item=denkhaus/work#1"}
        # 01PAINPOINT: the SAME visibility painpoint twice (an engine
        # retry re-recorded the first fact — the head collapses it) plus
        # a distinct Dockerfile-touch painpoint: TWO distinct painpoints.
        {runId: "01PAINPoinTRUN000000000000000", seq: 1, category: "painpoint", text: "fact=patch-visibility verdict=not-visible base=abc1234 item=denkhaus/work#2"}
        {runId: "01PAINPoinTRUN000000000000000", seq: 2, category: "painpoint", text: "fact=patch-visibility verdict=not-visible base=abc1234 item=denkhaus/work#2"}
        {runId: "01PAINPoinTRUN000000000000000", seq: 3, category: "painpoint", text: "fact=dockerfile-touch paths=.fabro/Dockerfile.toolchain"}
        # 01PARKED: the prior pass's fork context rides observations.
        {runId: "01PARKEDRUN00000000000000000", seq: 1, category: "note", text: "parked-fork: should the residual file as unfinished or exempt?"}
    ],
    radius: {
        "denkhaus/work#2": [{itemKey: "denkhaus/work#3", title: "The radius blocker", state: "open"}]
    }
}

# --- The sweep drive (engine-mode argv; the routing envelope must land)
let sweep = (do {
    with-env (shim-env $SHIM $state) {
        with-env {FABRO_RUN_ID: $ADJUDICATOR_RUN} {
            nu ($SCRIPTS_DIR | path join "fast-path.nu")
        }
    }
} | complete)
if $sweep.exit_code != 0 {
    fail $"the sweep must exit 0: ($sweep.stderr | str substring 0..300)"
}
let envelope = (try { $sweep.stdout | lines | compact | where {|l| $l | str starts-with '{'} | last | from json } catch { null })
if $envelope == null {
    fail $"the sweep must print its routing envelope JSON: ($sweep.stdout | str substring 0..300)"
}
if ($envelope | get -o preferred_next_label) != "Runs pending adjudication" {
    fail $"non-trivial runs must route to the agent pass: ($envelope | to json -r)"
}
let adjudication = ($envelope.context_updates | get -o 'output.adjudication')
if $adjudication == null {
    fail $"the envelope must carry output.adjudication: ($envelope | to json -r)"
}

# The trivial mass: split by pull request, marked directly.
let marks = (open --raw $state | from json | get -o marks | default [])
let merge_marks = ($marks | where {|m| $m.field == "merge" and $m.byRun == $ADJUDICATOR_RUN})
let enabled = ($merge_marks | where {|m| $m.verdict == "enabled"})
if ($enabled | length) != 1 or $enabled.0.runId != "01TRIVIALPR00000000000000000" {
    fail $"the trivial pull-request run must be the only enabled mark: ($merge_marks | to json -r)"
}
let refused = ($merge_marks | where {|m| $m.verdict == "refused"})
if ($refused | length) != 1 or $refused.0.runId != "01TRIVIALNOPR0000000000000000" {
    fail $"the trivial no-pull-request run must be the only refused mark: ($merge_marks | to json -r)"
}

# work#1072: the human_needed run parks DIRECTLY, LLM-free — the exit
# itself means a human decision pends. It carries a pull request and an
# empty radius: neither the PR split nor any fact read may run — the
# terminal kind alone routes.
let parked_human = ($merge_marks | where {|m| $m.verdict == "parked" and $m.runId == "01HUMANNEEDED00000000000000A"})
if ($parked_human | length) != 1 {
    fail $"the human_needed run must be parked directly by the sweep: ($merge_marks | to json -r)"
}
if ($adjudication.parked_human_needed | default []) != ["01HUMANNEEDED00000000000000A"] {
    fail $"the envelope names the human-needed parking: ($adjudication | to json -r)"
}

# The packets: painpoint run, parked run — never the trivial pair, never
# the smoke workflow's run, never the live run.
let packets = ($adjudication | get -o packets | default [])
let packet_ids = ($packets | get run_id)
if ($packets | length) != 2 {
    fail $"exactly the painpoint and parked runs become packets: ($packet_ids | to json -r)"
}
# work#1113: the radius read uses the ENTRY's project, never a hardcoded
# one — the painpoint run's claim rides a different project (fabro-test)
# and the shim's argv log must name it.
let radius_log = (open --raw $state | from json | get -o log | default [] | where {|l|
    ($l.argv | str join ' ') =~ 'radius'
})
let painpoint_radius = ($radius_log | where {|l| ($l.argv | str join ' ') =~ 'denkhaus/work#2' and $l.flags?.project? == "fabro-test"} | get -o 0 | default null)
if $painpoint_radius == null {
    fail $"the radius read must carry the entry's own project fabro-test, not a hardcode: ($radius_log | to json -r)"
}
let painpoint_packet = ($packets | where {|p| $p.run_id == "01PAINPoinTRUN000000000000000"} | get -o 0 | default null)
if $painpoint_packet == null { fail "the painpoint run must route to the agent pass" }
if ($painpoint_packet.painpoints | length) != 2 {
    fail $"the duplicate fact head must collapse to TWO distinct painpoints: ($painpoint_packet.painpoints | to json -r)"
}
if $painpoint_packet.work_item != "denkhaus/work#2" {
    fail $"the packet carries the claimed work item: ($painpoint_packet | to json -r)"
}
if $painpoint_packet.radius_items != 1 {
    fail $"the packet carries the open radius count: ($painpoint_packet | to json -r)"
}
if not $painpoint_packet.has_pull_request { fail "the packet carries the pull-request state" }
let parked_packet = ($packets | where {|p| $p.run_id == "01PARKEDRUN00000000000000000"} | get -o 0 | default null)
if $parked_packet == null { fail "the parked run must re-enter as a packet" }
if $parked_packet.verdict != "parked" {
    fail $"the parked packet names its escalation state: ($parked_packet | to json -r)"
}
# The fast path never re-marks a parked run (the agent pass owns it).
if (($merge_marks | where {|m| $m.runId == "01PARKEDRUN00000000000000000"}) | length) != 0 {
    fail "the fast path must not re-mark a parked run"
}
# The workflow filter kept the smoke run out; the live run never queued.
if ("01SMOKERUN000000000000000000" in $packet_ids) { fail "the smoke workflow's run leaked through the filter" }
if ("01LIVERUN00000000000000000000" in ($marks | get runId)) { fail "a live run was marked" }

# --- The empty-worklist drive: everything the fast path marks is
# settled; the packets' runs stay queued (the agent pass owns them), so
# the label stays non-empty — but a state with NO queued runs routes
# "Queue empty".
let empty_state = (mktemp -t adjudicator-empty.XXXXXX.json)
write-shim-state $empty_state {items: [], claims: [], nextKey: 9000, log: [], runs: [], marks: [], observations: [], radius: {}}
let empty_run = (do {
    with-env (shim-env $SHIM $empty_state) {
        nu ($SCRIPTS_DIR | path join "fast-path.nu")
    }
} | complete)
if $empty_run.exit_code != 0 {
    fail $"the empty sweep must exit 0: ($empty_run.stderr | str substring 0..240)"
}
if not ($empty_run.stdout | str contains '"Queue empty"') {
    fail $"an empty worklist must route Queue empty: ($empty_run.stdout | str substring 0..240)"
}

# --- Negative arm: a missing FABRO_SERVER is refused loudly.
let no_server = (do {
    with-env {FABRO_BIN: $SHIM, FABRO_WORK_SHIM_STATE: $state} {
        hide-env FABRO_SERVER
        nu ($SCRIPTS_DIR | path join "fast-path.nu")
    }
} | complete)
if $no_server.exit_code != 1 { fail "a missing FABRO_SERVER must be refused loudly (exit 1)" }

# --- Hard-transport arm (#1132): a CLI that answers non-zero fails the
# stage — the read-queue-hard wrapper's shared policy for all three
# pass scripts. /bin/false stands in for a dying binary; the sweep must
# exit 1 naming the queue read, never swallow the failure.
let dead_bin = (do {
    with-env {FABRO_BIN: "/bin/false", FABRO_SERVER: "http://127.0.0.1:1"} {
        nu ($SCRIPTS_DIR | path join "fast-path.nu")
    }
} | complete)
if $dead_bin.exit_code != 1 {
    fail $"a non-zero queue read must fail the stage: exit ($dead_bin.exit_code)"
}
if not ($dead_bin.stderr | str contains "run queue read failed") {
    fail $"the hard wrapper must name the failed read: ($dead_bin.stderr | str substring 0..160)"
}

# --- The real-parser argv pin (the closeout-smoke precedent): the shim
# is position-blind, so every outbound argv is pinned against the REAL
# binary on a dead server — a clap parse error ('unexpected argument')
# is distinguishable from the expected transport failure. The run
# facade's grammar is verb-first (the workflow positional would eat a
# leading subcommand after a flag).
do {
    let tree_bin = ($SCRIPTS_DIR | path join '..' '..' '..' '..' 'target' 'debug' 'fabro')
    let fabro_bin = (if ($tree_bin | path exists) { $tree_bin } else { which fabro | get 0.path })
    let dead = "http://127.0.0.1:1"
    pin-argv $fabro_bin $dead [run queue --field merge --workflow develop --json --server $dead]
    pin-argv $fabro_bin $dead [run mark "01M5844J1FDD" merge=enabled --server $dead]
    pin-argv $fabro_bin $dead [observation --server $dead list --run-id "01M5844J1FDD" --json]
    pin-argv $fabro_bin $dead [work --server $dead radius --project fabro "denkhaus/work#1" --json]
    pin-argv $fabro_bin $dead [work --server $dead comments --project fabro "denkhaus/work#1" --json]
    print 'ok: every outbound argv parses against the real CLI grammar'
}

rm -f $state $empty_state
end-fail-gate
print "adjudicator-fast-path-smoke: ok — trivial marks split by pull request, packets for painpoint/radius/parked runs, fact-head dedupe, filter, routing envelope"

# Sourcing fast-path.nu imports its `def main`; nu auto-invokes it after
# the top level runs — exit explicitly so the smoke never reaches it.
exit 0
