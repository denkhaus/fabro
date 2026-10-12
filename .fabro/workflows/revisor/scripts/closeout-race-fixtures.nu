#!/usr/bin/env nu
# Fixture battery for closeout-race.nu (fabro-74d4): the closeout race —
# the target run's own closeout files seeds between the revisor's
# analyze-time snapshot and its filing stage — must be mechanically
# visible. Fixtures pin the listing logic over a scratch tracker JSONL
# (no live seeds call for the core cases) plus the PROMPT-ORDERING pin
# the seed's acceptance criterion demands: analyze dedupes against the
# closeout listing before distilling, and the bookkeeper re-runs it at
# filing time treating a match as a duplicate (never an overflow).
#
# Cases:
#   (a) listing includes exactly the seeds whose title/body cite the
#       target run, from either field; unrelated seeds are excluded
#   (b) --since keeps only seeds updated strictly after the watermark
#       (the race-window view: closeout filings land AFTER the snapshot)
#   (c) a malformed tracker line is tolerated, not fatal
#   (d) a missing tracker file degrades fail-open (outcome degraded,
#       exit 0 — a guard's own failure never blocks a pass)
#   (e) prompt pin: analyze.md routes the listing BEFORE distillation
#       with a covered_by_closeout drop rule
#   (f) prompt pin: file.md re-runs the listing at FILING time and
#       classifies a match as duplicate_of, never an overflow entry
#
# Usage: nu .fabro/workflows/revisor/scripts/closeout-race-fixtures.nu
# (exit 0 = all pass)

# prompt-lint: synthetic-seed-ids

const SCRIPT = ('.fabro/workflows/revisor/scripts/closeout-race.nu' | path expand)
const ANALYZE = ('.fabro/workflows/revisor/prompts/analyze.md' | path expand)
const FILE = ('.fabro/workflows/revisor/prompts/file.md' | path expand)

const TARGET = '01MFIXTURERUN000000000AX'
const OTHER = '01MFIXTUREOTHER00000000BX'

def fail [what: string]: nothing -> nothing {
    print -e $"closeout-race-fixtures: FAIL — ($what)"
    exit 1
}

def expect [case: string, got, want] {
    if $got == $want {
        print $"PASS \($case\): ($got)"
    } else {
        print -e $"FAIL \($case\): got ($got), want ($want)"
        exit 1
    }
}

# Run the script against a scratch tracker and return {exit, json}.
def run-listing [tracker: string, since?: string] {
    let args = if $since != null { [$TARGET '--tracker' $tracker '--since' $since] } else { [$TARGET '--tracker' $tracker] }
    let r = (do { nu $SCRIPT ...$args } | complete)
    {exit: $r.exit_code, json: (try { $r.stdout | str trim | from json } catch { null }), stderr: $r.stderr}
}

def main [] {
    if not ($SCRIPT | path exists) { fail $"script not found: ($SCRIPT)" }
    let tmp = (mktemp -d -t 'closeout-race-fixtures.XXXXXX')
    let tracker = ($tmp | path join 'issues.jsonl')

    # Scratch tracker: a closeout seed citing the target in its Basis
    # (created after the watermark), a title-citer updated before it, an
    # unrelated seed, and one malformed line.
    [
        {id: 'fabro-0aaa', title: 'env_guard sibling wiring', status: 'open', createdAt: '2026-10-04T14:45:30.811Z', updatedAt: '2026-10-04T14:45:30.811Z', description: $"closeout follow-up. Basis: run ($TARGET), workflow version absent, commit deadbeef"}
        {id: 'fabro-0bbb', title: $"review of run ($TARGET)", status: 'closed', createdAt: '2026-10-01T00:00:00.000Z', updatedAt: '2026-10-01T00:00:00.000Z', description: 'older seed citing the run only in title'}
        {id: 'fabro-0ccc', title: 'unrelated', status: 'open', createdAt: '2026-10-02T00:00:00.000Z', updatedAt: '2026-10-02T00:00:00.000Z', description: $"cites a different run ($OTHER), not the target"}
        'this line is not json'
    ]
    | each {|row| if ($row | describe | str starts-with 'record') { $row | to json -r } else { $row } }
    | str join (char newline)
    | save -f $tracker

    # (a) full listing: exactly the two citers, in tracker order.
    let a = (run-listing $tracker)
    if $a.exit != 0 { fail $"case a exited ($a.exit): ($a.stderr)" }
    if $a.json == null { fail 'case a produced no JSON' }
    if $a.json.outcome != 'ok' { fail $"case a outcome ($a.json.outcome) != ok" }
    expect 'a: matching ids' ($a.json.matching | get id) ['fabro-0aaa' 'fabro-0bbb']
    expect 'a: count' $a.json.count 2

    # (b) watermark: only the closeout seed updated after the snapshot.
    let b = (run-listing $tracker '2026-10-04T00:00:00.000Z')
    if $b.exit != 0 { fail $"case b exited ($b.exit): ($b.stderr)" }
    expect 'b: race-window ids' ($b.json.matching | get id) ['fabro-0aaa']
    expect 'b: count' $b.json.count 1

    # (c) malformed line tolerated — already proven by (a)/(b) counting
    # past it; pin it explicitly: the run stayed ok with 2 matches.
    expect 'c: malformed line tolerated (outcome ok)' $a.json.outcome 'ok'

    # (d) missing tracker file degrades fail-open, exit 0.
    let d = (run-listing ($tmp | path join 'does-not-exist.jsonl'))
    expect 'd: degraded exit code' $d.exit 0
    expect 'd: degraded outcome' $d.json.outcome 'degraded'

    # (e) analyze pin: the listing call routes BEFORE distillation, with
    # a covered_by_closeout drop rule.
    let analyze = (open --raw $ANALYZE)
    if not ($analyze | str contains 'closeout-race.nu') { fail 'analyze.md does not call closeout-race.nu — the analyst can still dedupe against a stale snapshot' }
    if not ($analyze | str contains 'revisor_target_run_id') { fail 'analyze.md closeout call must use the target run id from context' }
    if not ($analyze | str contains 'covered_by_closeout') { fail 'analyze.md must name the covered_by_closeout drop record' }

    # (f) file pin: the bookkeeper re-runs the listing at FILING time and
    # a match is a duplicate, never an overflow entry.
    let file = (open --raw $FILE)
    if not ($file | str contains 'closeout-race.nu') { fail 'file.md does not re-run closeout-race.nu — the race window between analyze and filing stays open' }
    if not ($file | str contains 'duplicate_of') { fail 'file.md must classify a closeout-race match as duplicate_of' }
    if not ($file | str contains 'NEVER an overflow') { fail 'file.md must pin that a closeout-covered finding is never journaled as overflow' }

    rm -rf $tmp
    print 'closeout-race-fixtures: OK — listing, watermark, tolerance, fail-open, and both prompt pins hold'
}
