#!/usr/bin/env nu
# Smoke test for the watchdog stall check (ADR-0039 stage F,
# denkhaus/work#1107): drives stall-check.nu end to end through the
# channel shim — the three grilled arms: a DISABLED gear stays silent
# (the pause is authoritative), an enabled gear with fresh completed
# passes is healthy, an enabled gear whose last completed pass exceeds
# 3x its tick interval ALARMS (exit non-zero, the run fails, the Slack
# lifecycle notification pings). No live server: the shim fakes the
# automations and ps doors.

const SCRIPTS_DIR = (path self | path dirname)

use ../../../scripts/lib/smoke_harness.nu [start-fail-log, fail, end-fail-gate, write-shim-state, drive]
load-env {SMOKE_FAIL_LOG: (start-fail-log 'watchdog-stall-check-smoke')}

# --- Arm 1: no automations at all (the pre-window posture) — every
# gear reads disabled, the check exits 0 with the healthy line. The
# pause is authoritative: silence, never an alarm. The pass titles the
# run with the no-enabled-gears outcome (#1159 follow-through).
let watchdog_own = "01WATCHDOG000000000000000A1"
let silent_state = (mktemp -t watchdog-silent.XXXXXX.json)
write-shim-state $silent_state {items: [], claims: [], nextKey: 9100, log: [], marks: [], observations: [], radius: {}, automations: [], runs: [
    {runId: $watchdog_own, workflowSlug: "watchdog", status: "running", startedMinutesAgo: 0}
]}
let silent = (drive ($SCRIPTS_DIR | path join "stall-check.nu") $silent_state [] $watchdog_own)
if $silent.exit_code != 0 {
    fail $"no automations must stay silent and green: ($silent.stderr | str substring 0..300)"
}
if not ($silent.stdout | str contains 'All gears healthy') {
    fail $"the healthy routing line must print: ($silent.stdout | str substring 0..300)"
}
let silent_title = (open --raw $silent_state | from json | get runs | where runId == $watchdog_own | get 0.title)
if not ($silent_title | str contains 'no enabled gears') {
    fail $"the silent pass must title the run with the no-enabled-gears outcome: ($silent_title)"
}

# --- Arm 2: every gear enabled with FRESH completed passes inside its
# window (adjudicator 10m < 45m, merger 20m < 90m, develop 30m < 90m)
# — healthy, exit 0, every gear named.
let healthy_state = (mktemp -t watchdog-healthy.XXXXXX.json)
write-shim-state $healthy_state {
    items: [], claims: [], nextKey: 9101, log: [], marks: [], observations: [], radius: {},
    automations: [
        {workflow: "adjudicator", scheduleEnabled: true}
        {workflow: "merger", scheduleEnabled: true}
        {workflow: "develop", scheduleEnabled: true}
    ],
    runs: [
        {runId: "01ADJPASS00000000000000000A", workflowSlug: "adjudicator", status: "succeeded", startedMinutesAgo: 10}
        {runId: "01MERGPASS0000000000000000A", workflowSlug: "merger", status: "succeeded", startedMinutesAgo: 20}
        {runId: "01DEVPASS000000000000000000A", workflowSlug: "develop", status: "succeeded", startedMinutesAgo: 30}
        # A STILL-RUNNING pass does not count: only a COMPLETED pass
        # proves the gear works.
        {runId: "01ADJLIVE000000000000000000A", workflowSlug: "adjudicator", status: "running", startedMinutesAgo: 1}
        # The watchdog's OWN run: the set-title target (FABRO_RUN_ID).
        {runId: "01WATCHDOG000000000000000A2", workflowSlug: "watchdog", status: "running", startedMinutesAgo: 0}
    ],
}
let healthy = (drive ($SCRIPTS_DIR | path join "stall-check.nu") $healthy_state [] "01WATCHDOG000000000000000A2")
if $healthy.exit_code != 0 {
    fail $"fresh passes must stay healthy: ($healthy.stderr | str substring 0..300)"
}
for gear in [adjudicator triager merger develop] {
    if not ($healthy.stdout | str contains $gear) {
        fail $"the healthy line must name ($gear): ($healthy.stdout | str substring 0..300)"
    }
}
let healthy_title = (open --raw $healthy_state | from json | get runs | where runId == "01WATCHDOG000000000000000A2" | get 0.title)
if not ($healthy_title | str contains 'all enabled gears healthy') {
    fail $"the healthy pass must title the run with the healthy outcome: ($healthy_title)"
}

# --- Arm 3: adjudicator STALLED — enabled, but its last COMPLETED pass
# sits 60m back, past the 45m window (the fresh running pass must not
# mask it); merger and develop stay healthy. The check ALARMS: exit 1,
# the stall names adjudicator and the age.
let stalled_state = (mktemp -t watchdog-stalled.XXXXXX.json)
write-shim-state $stalled_state {
    items: [], claims: [], nextKey: 9102, log: [], marks: [], observations: [], radius: {},
    automations: [
        {workflow: "adjudicator", scheduleEnabled: true}
        {workflow: "merger", scheduleEnabled: true}
        {workflow: "develop", scheduleEnabled: true}
    ],
    runs: [
        {runId: "01ADJSTALE0000000000000000A", workflowSlug: "adjudicator", status: "succeeded", startedMinutesAgo: 60}
        {runId: "01ADJLIVE2000000000000000AA", workflowSlug: "adjudicator", status: "running", startedMinutesAgo: 2}
        {runId: "01MERGOK000000000000000000A", workflowSlug: "merger", status: "succeeded", startedMinutesAgo: 25}
        {runId: "01DEVOK0000000000000000000A", workflowSlug: "develop", status: "succeeded", startedMinutesAgo: 35}
        # The watchdog's OWN run: the set-title target (FABRO_RUN_ID).
        {runId: "01WATCHDOG000000000000000A3", workflowSlug: "watchdog", status: "running", startedMinutesAgo: 0}
    ],
}
let stalled = (drive ($SCRIPTS_DIR | path join "stall-check.nu") $stalled_state [] "01WATCHDOG000000000000000A3")
if $stalled.exit_code == 0 {
    fail "a stalled gear must exit non-zero — the red run IS the alarm"
}
if not ($stalled.stderr | str contains 'GEAR STALL') {
    fail $"the alarm must say GEAR STALL: ($stalled.stderr | str substring 0..300)"
}
if not ($stalled.stderr | str contains 'adjudicator') {
    fail $"the alarm must name the stalled gear: ($stalled.stderr | str substring 0..300)"
}
let stalled_title = (open --raw $stalled_state | from json | get runs | where runId == "01WATCHDOG000000000000000A3" | get 0.title)
if not ($stalled_title | str contains 'Watchdog STALL') {
    fail $"the stall pass must title the run with the STALL outcome: ($stalled_title)"
}
if not ($stalled_title | str contains 'adjudicator') {
    fail $"the stall title must name the stalled gear: ($stalled_title)"
}
# A disabled gear never rides along into the alarm (arm 1's semantics
# inside the stall drive): remove merger's automation and the alarm
# still names ONLY adjudicator.
let mixed_state = (mktemp -t watchdog-mixed.XXXXXX.json)
write-shim-state $mixed_state {
    items: [], claims: [], nextKey: 9103, log: [], marks: [], observations: [], radius: {},
    automations: [
        {workflow: "adjudicator", scheduleEnabled: true}
        {workflow: "develop", scheduleEnabled: true}
    ],
    runs: [
        {runId: "01ADJSTALE2000000000000000A", workflowSlug: "adjudicator", status: "succeeded", startedMinutesAgo: 60}
        {runId: "01DEVOK20000000000000000A", workflowSlug: "develop", status: "succeeded", startedMinutesAgo: 35}
    ],
}
let mixed = (drive ($SCRIPTS_DIR | path join "stall-check.nu") $mixed_state)
if $mixed.exit_code == 0 {
    fail "the stalled adjudicator must alarm even with merger absent"
}
if ($mixed.stderr | str contains 'merger') {
    fail $"an absent automation is silent, never an alarm: ($mixed.stderr | str substring 0..300)"
}

# --- Arm 4: the graph's RED conclusion is pinned AT the stage
# (denkhaus/work#1168 fold, the code-review demand): arms 1-3 assert the
# SCRIPT's exit; this arm pins the GRAPH mechanism that turns a stall
# into a red run. The `outcome=failed` conditional edge is an ABSORBER —
# the conclusion fold consumes it as declared control flow and concludes
# the run green at exit (the pinned petri semantics, work#1029 class; run
# 01M7QBW8A10DJ9YKJ2EH46EQG8 absorbed a live GEAR STALL that way). The
# landed graph routes red over `on_failure="exit"` on the stall_check
# node (the stage itself fails the run, classified failed AT the stage)
# and keeps the healthy preferred_label conditional as the only
# stall_check condition edge. No cargo, no engine: the pin reads the
# graph text.
let graph_path = ($SCRIPTS_DIR | path join '..' 'workflow.fabro')
let graph_lines = (open --raw $graph_path | lines)
let stall_at = ($graph_lines | enumerate | where {|it| $it.item | str contains 'stall_check ['} | get -o 0 | default null)
if $stall_at == null {
    fail 'the watchdog graph lost the stall_check node — the dead-man watch has no stage'
}
mut stall_block = []
for it in ($graph_lines | enumerate) {
    if $it.index >= $stall_at.index {
        if ($it.item | str trim) == ']' { break }
        $stall_block = ($stall_block | append $it.item)
    }
}
if not ($stall_block | any {|l| $l | str contains 'on_failure="exit"'}) {
    fail 'the stall_check node lost on_failure="exit" — a stall would route onward instead of reddening the run AT the stage (work#1168)'
}
let stall_edges = ($graph_lines | where {|l| ($l | str trim | str starts-with 'stall_check ->') })
if ($stall_edges | length) < 2 {
    fail 'the stall_check node lost its exit edges — the healthy route and the soft fallback must both exist'
}
let absorber = ($stall_edges | where {|l| $l | str contains 'condition="outcome' })
if ($absorber | is-not-empty) {
    for line in $absorber {
        print -e $"the absorber edge class regrew in workflow.fabro: ($line | str trim)"
    }
    fail 'an outcome=... conditional edge on stall_check is an ABSORBER — the engine concludes the run green at exit over it (work#1029 class, work#1168); route red via on_failure=exit instead'
}
if not ($stall_edges | any {|l| $l | str contains 'preferred_label=\"All gears healthy\"'}) {
    fail 'the healthy preferred_label conditional edge is gone — green must route explicitly, not by accident'
}

end-fail-gate
print 'watchdog-stall-check-smoke: ok — silent when disabled, healthy when fresh, GEAR STALL alarm past the window, red pinned AT the stage'
