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
# pause is authoritative: silence, never an alarm.
let silent_state = (mktemp -t watchdog-silent.XXXXXX.json)
write-shim-state $silent_state {items: [], claims: [], nextKey: 9100, log: [], runs: [], marks: [], observations: [], radius: {}, automations: []}
let silent = (drive ($SCRIPTS_DIR | path join "stall-check.nu") $silent_state)
if $silent.exit_code != 0 {
    fail $"no automations must stay silent and green: ($silent.stderr | str substring 0..300)"
}
if not ($silent.stdout | str contains 'All gears healthy') {
    fail $"the healthy routing line must print: ($silent.stdout | str substring 0..300)"
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
    ],
}
let healthy = (drive ($SCRIPTS_DIR | path join "stall-check.nu") $healthy_state)
if $healthy.exit_code != 0 {
    fail $"fresh passes must stay healthy: ($healthy.stderr | str substring 0..300)"
}
for gear in [adjudicator merger develop] {
    if not ($healthy.stdout | str contains $gear) {
        fail $"the healthy line must name ($gear): ($healthy.stdout | str substring 0..300)"
    }
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
    ],
}
let stalled = (drive ($SCRIPTS_DIR | path join "stall-check.nu") $stalled_state)
if $stalled.exit_code == 0 {
    fail "a stalled gear must exit non-zero — the red run IS the alarm"
}
if not ($stalled.stderr | str contains 'GEAR STALL') {
    fail $"the alarm must say GEAR STALL: ($stalled.stderr | str substring 0..300)"
}
if not ($stalled.stderr | str contains 'adjudicator') {
    fail $"the alarm must name the stalled gear: ($stalled.stderr | str substring 0..300)"
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

end-fail-gate
print 'watchdog-stall-check-smoke: ok — silent when disabled, healthy when fresh, GEAR STALL alarm past the window'
