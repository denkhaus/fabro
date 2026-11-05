#!/usr/bin/env nu
# The dead-man stall check (ADR-0039 stage F, denkhaus/work#1107; grilled
# 2026-11-05): every ENABLED scheduler gear must have COMPLETED a pass
# within 3x its tick interval. A gear whose automation is disabled (or
# absent) stays SILENT — the user's pause is authoritative and never
# overridden. An enabled gear with no completed pass in its window is a
# STALL: this script exits non-zero, the watchdog run fails, and the
# Slack lifecycle notification carries the alarm to the operator.
#
# The stall windows derive from the grilled cron shapes (epic #1107
# stage-F design): adjudicator */15 -> 45m, merger 10,40 -> 90m,
# develop 20,50 -> 90m. K=3 consecutive ticks without a completed pass.
#
# FACT FAILURE DISCIPLINE: a door read that fails FAILS THE STAGE — for
# the watchdog a failed read IS alarm-worthy (the server unreachable
# from the run sandbox is itself a stall signal).
#
# META-STALL (grilled: list visibility): nothing here watches the
# watchdog itself. Its automation row (last fire, last error) and
# `fabro ps -a --workflow watchdog` make a dead watchdog visible on the
# operator's manual glance.

use ../../../scripts/lib/work_channel.nu [server, cli]

const CALLER = 'watchdog-stall-check'

# The gears and their stall windows (3x the tick interval, K=3).
const GEARS = [
    [workflow interval_minutes window_minutes];
    [adjudicator 15 45]
    [merger 30 90]
    [develop 30 90]
]

# The automation rows of the project (the gear automations live on the
# same server; name shape `<workflow>@fabro`). HARD on transport
# failure.
def automation-rows []: nothing -> list<any> {
    let srv = (server $CALLER)
    let res = (cli $CALLER [automations --server $srv list --json])
    if $res.exit_code != 0 {
        print -e $"($CALLER): automations list failed: ($res.stderr | str trim | str substring 0..240)"
        exit 1
    }
    try { $res.stdout | from json } catch {
        print -e $"($CALLER): automations list printed no JSON: ($res.stdout | str trim | str substring 0..240)"
        exit 1
    }
}

# Whether the workflow's gear automation has its SCHEDULE trigger
# enabled. Absent automation = not enabled (the pre-window posture: the
# gears go live at the fork.30 cutover, stage G).
def schedule-enabled [rows: list<any>, workflow: string]: nothing -> bool {
    let row = ($rows | where {|r| ($r | get -o workflow | default '') == $workflow} | get -o 0 | default null)
    if $row == null { return false }
    let schedule = ($row | get -o triggers | default [] | where {|t| ($t | get -o type | default '') == 'schedule'} | get -o 0 | default null)
    if $schedule == null { return false }
    ($schedule | get -o enabled | default false)
}

# The age (minutes) of the workflow's newest TERMINAL run — its last
# completed pass. null when the workflow has none. HARD on transport
# failure. A pass that started within the window but still runs does
# not count: only a COMPLETED pass proves the gear works.
def last-pass-age-minutes [workflow: string]: nothing -> any {
    let srv = (server $CALLER)
    let res = (cli $CALLER [ps --server $srv --all --json --workflow $workflow])
    if $res.exit_code != 0 {
        print -e $"($CALLER): ps failed for ($workflow): ($res.stderr | str trim | str substring 0..240)"
        exit 1
    }
    let runs = (try { $res.stdout | from json } catch {
        print -e $"($CALLER): ps printed no JSON for ($workflow): ($res.stdout | str trim | str substring 0..240)"
        exit 1
    })
    let terminal = ($runs | where {|r|
        let kind = ($r | get -o status | get -o kind | default '')
        $kind in ['succeeded' 'failed' 'human_needed' 'dead']
    })
    let newest = ($terminal | sort-by start_time --reverse | get -o 0 | default null)
    if $newest == null { return null }
    let started = ($newest | get -o start_time | default '' | into datetime)
    (((date now) - $started) / 1min | into int)
}

def main []: nothing -> nothing {
    let rows = (automation-rows)
    mut stalls = []
    mut healthy = []
    for gear in $GEARS {
        let enabled = (schedule-enabled $rows $gear.workflow)
        if not $enabled {
            # The pause is authoritative: a disabled gear never alarms,
            # even with work waiting (grilled stall semantics).
            print $"($CALLER): ($gear.workflow): automation disabled — silent, the pause is authoritative"
            continue
        }
        let age = (last-pass-age-minutes $gear.workflow)
        if $age == null {
            $stalls = ($stalls | append {gear: $gear.workflow, detail: 'enabled but NEVER completed a pass'})
            continue
        }
        if $age > $gear.window_minutes {
            $stalls = ($stalls | append {gear: $gear.workflow, detail: $"last completed pass ($age)min ago, window ($gear.window_minutes)min"})
        } else {
            $healthy = ($healthy | append $"($gear.workflow)\(($age)min\)")
        }
    }
    if ($stalls | is-empty) {
        print $"($CALLER): all enabled gears healthy: ($healthy | str join ', ')"
        print 'preferred_label: "All gears healthy"'
        return
    }
    # THE ALARM: exit non-zero, name every stalled gear. The run fails;
    # the Slack lifecycle notification pings the operator.
    print -e $"($CALLER): GEAR STALL — the dead-man watch alarms:"
    for stall in $stalls {
        print -e $"($CALLER):   ($stall.gear): ($stall.detail)"
    }
    exit 1
}
