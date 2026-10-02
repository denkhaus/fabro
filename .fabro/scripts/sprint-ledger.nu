#!/usr/bin/env nu
# Sprint ledger (fabro-cadd, ADR-0024 nu-agent sprint-model port): the
# committed state file `.fabro/iterate-state.json` is the loop line's
# sprint counter + reflection invariant. This script is its ONLY writer
# and its deterministic check — prompts never hand-edit the ledger.
#
# Model (settled 2026-10-02, closing the seed's design fork):
#   - ONE closed seed whose run diff (claim/run-base anchored, tracker
#     `.seeds/issues.jsonl` and `.fabro/journal/**` bookkeeping excluded)
#     is non-empty counts 1 sprint, in EVERY lane. Verify-only /
#     bookkeeping-only closures count 0. The earlier 1/3-per-loop-seed
#     idea is dropped: weighting by diff substance is mechanical and
#     lane-blind, and no cadence data argued for fractional counting.
#   - Reflection invariant (fail-closed park): no new pass may start
#     while sprints_reflected < sprints_completed. `record-close`
#     increments sprints_completed ONLY — the deliberate reflection act
#     (`reflect`, driven by the session-side line-watch per the iterate
#     skill's Phase 6; out of this file's hands) closes the gap. The
#     workflow's tracker-guard parks the next run until it lands.
#   - Architecture gate (era note 2026-10-02): when sprints_completed
#     % 3 == 0 and last_arch_review_at_sprint != sprints_completed,
#     `record-close` prints an arch-gate-DUE advisory. Firing the LOCAL
#     improve-codebase-architecture agent is session-side work; the
#     architect WORKFLOW stays deactivated on the line (user directive)
#     and auto-fire is future scope.
#
# FAIL-OPEN contract: every internal error (unreadable/unparseable
# ledger, git failure) degrades to a no-op that prints a warning and
# exits 0 — the ledger must never block a close or dead-end a run.
# `check` under failure reports park=false, degraded=true (the park is
# fail-closed only on PROVEN unreflected state, mirroring tracker-guard).
#
# Usage:
#   nu .fabro/scripts/sprint-ledger.nu check
#   nu .fabro/scripts/sprint-ledger.nu record-close --seed <id> [--run-id <id>]
#   nu .fabro/scripts/sprint-ledger.nu reflect [--note <text>]
#   nu .fabro/scripts/sprint-ledger.nu arch-review-done   # after a gate pass

const LEDGER = ".fabro/iterate-state.json"

# Bookkeeping-only paths never count as sprint substance.
def bookkeeping? [p: string]: nothing -> bool {
    if $p == ".seeds/issues.jsonl" { return true }
    ($p | str starts-with ".fabro/journal/")
}

# Pure filter over a diff path list: non-bookkeeping paths remain.
def substantive-paths [paths: list<string>]: nothing -> list<string> {
    $paths | compact | where {|p| not (bookkeeping? $p) }
}

# Parsed ledger or null (missing/unparseable). Pure over the raw text so
# the smoke can exercise it without touching the real file.
def parse-ledger [text: string]: nothing -> any {
    let parsed = (try { $text | from json } catch { null })
    if not ($parsed | describe | str starts-with "record") { return null }
    if (($parsed.sprints_completed? | default null) == null) { return null }
    if (($parsed.sprints_reflected? | default null) == null) { return null }
    $parsed
}

# Park verdict over a parsed ledger record (pure, smoke-testable):
# park=true iff the reflection invariant is PROVEN violated.
def park-decision [ledger: any]: nothing -> record {
    if $ledger == null {
        {park: false, degraded: true, sprints_completed: null, sprints_reflected: null}
    } else if ($ledger.sprints_reflected < $ledger.sprints_completed) {
        {park: true, degraded: false, sprints_completed: $ledger.sprints_completed, sprints_reflected: $ledger.sprints_reflected}
    } else {
        {park: false, degraded: false, sprints_completed: $ledger.sprints_completed, sprints_reflected: $ledger.sprints_reflected}
    }
}

def read-ledger []: nothing -> any {
    if not ($LEDGER | path exists) { return null }
    parse-ledger (open --raw $LEDGER)
}

def write-ledger [ledger: record]: nothing -> nothing {
    $ledger | to json | save --force $LEDGER
}

def ledger-notes [ledger: record]: nothing -> list<string> {
    let notes = ($ledger | get -o notes | default [])
    if ($notes | describe | str starts-with "list") { $notes } else { [] }
}

# Run diff base: the run-branch checkpoint idiom (last
# `fabro(<run-id>):` checkpoint's parent; HEAD-anchored fallback).
def run-base []: nothing -> string {
    let run_id = (
        do { git branch --show-current } | complete | get stdout | str trim
        | parse --regex 'fabro/run/(?P<id>[^/]+)$'
        | get -o id.0
        | default ''
    )
    if ($run_id | is-empty) { return "HEAD" }
    let subject_mark = $"fabro\(($run_id)\):"
    let checkpoints = (do { git log --format=%H --fixed-strings --grep $subject_mark } | complete | get stdout | lines | compact)
    if ($checkpoints | is-empty) { return "HEAD" }
    do { git rev-parse $"($checkpoints | last)^" } | complete | get stdout | str trim
}

def today []: nothing -> string {
    (date now | format date "%Y-%m-%d")
}

def fail-open [what: string]: nothing -> nothing {
    print -e $"sprint-ledger: WARNING — \($what\) \(ledger left untouched, fail-open\)"
}

# Park check: prints the routing verdict the tracker-guard consumes.
def do-check []: nothing -> nothing {
    let dec = (park-decision (read-ledger))
    if $dec.park {
        print -e $"sprint-ledger: PARK — sprints_reflected \($dec.sprints_reflected\) < sprints_completed \($dec.sprints_completed\): run the session-side short reflection \(iterate Phase 6\) or `nu .fabro/scripts/sprint-ledger.nu reflect` before the next pass."
    }
    $dec | to json --raw | print
}

# Record a closed seed: +1 sprint when the run diff was substantive,
# never touching sprints_reflected; prints the arch-gate advisory when
# due. Substantive = at least one non-bookkeeping diff path against the
# run base (git failures degrade to counting the close — a closed seed
# with an unreadable diff is more likely substantive than not, and the
# reflection park still applies either way).
def do-record-close [seed_id: string, run_id: string]: nothing -> nothing {
    let ledger = (read-ledger)
    if $ledger == null { fail-open "ledger missing/unparseable"; return }
    let diff_res = (do { git diff --name-only (run-base) } | complete)
    let substantive = (if $diff_res.exit_code != 0 {
        true
    } else {
        (substantive-paths ($diff_res.stdout | lines) | is-not-empty)
    })
    if not $substantive {
        print $"sprint-ledger: \($seed_id\) closed with a bookkeeping-only diff — counts 0 sprints \(ledger unchanged\)"
        return
    }
    let n = ($ledger.sprints_completed + 1)
    let gate_due = ($n mod 3 == 0 and ($ledger.last_arch_review_at_sprint? | default 0) != $n)
    let run_part = (if ($run_id | is-empty) { "" } else { $", run \($run_id)" })
    let note = $"sprint \($n\) \($seed_id\)\($run_part) closed \((date now | format date '%Y-%m-%d')\) — counted by sprint-ledger record-close; reflection pending"
    let updated = ($ledger
        | upsert sprints_completed $n
        | upsert notes {|l| (ledger-notes $l | append [$note])}
    )
    try { write-ledger $updated } catch {|e| fail-open $"ledger write failed: \($e.msg\)"; return }
    print $"sprint-ledger: \($seed_id\) counted as sprint \($n\); reflection now pending \(park until reflect\)"
    if $gate_due {
        print -e $"sprint-ledger: ARCH GATE DUE at sprint \($n\) — fire the LOCAL improve-codebase-architecture agent \(era note: the architect workflow stays off the line\); after the pass run `sprint-ledger arch-review-done`"
    }
}

# Reflection act: closes the invariant gap.
def do-reflect [note: string]: nothing -> nothing {
    let ledger = (read-ledger)
    if $ledger == null { fail-open "ledger missing/unparseable"; return }
    let updated = ($ledger
        | upsert sprints_reflected $ledger.sprints_completed
        | upsert last_reflection (today)
        | upsert notes {|l| (ledger-notes $l | append (if ($note | is-empty) { [] } else { [$note] }))}
    )
    try { write-ledger $updated } catch {|e| fail-open $"ledger write failed: \($e.msg\)"; return }
    print $"sprint-ledger: reflected — sprints_reflected = \($updated.sprints_reflected), last_reflection = \($updated.last_reflection)"
}

# Mark the architecture gate as run at the current sprint boundary.
def do-arch-review-done []: nothing -> nothing {
    let ledger = (read-ledger)
    if $ledger == null { fail-open "ledger missing/unparseable"; return }
    let updated = ($ledger | upsert last_arch_review_at_sprint $ledger.sprints_completed)
    try { write-ledger $updated } catch {|e| fail-open $"ledger write failed: \($e.msg\)"; return }
    print $"sprint-ledger: arch review recorded at sprint \($updated.sprints_completed)"
}

def main [--seed: string = "", --run-id: string = "", --note: string = "", cmd: string = "check"]: nothing -> nothing {
    match $cmd {
        "check" => { do-check }
        "record-close" => {
            if ($seed | is-empty) { print -e "sprint-ledger: record-close requires --seed"; exit 0 }
            do-record-close $seed $run_id
        }
        "reflect" => { do-reflect $note }
        "arch-review-done" => { do-arch-review-done }
        _ => { print -e $"sprint-ledger: unknown subcommand '\($cmd\)' \(check | record-close | reflect | arch-review-done\)"; exit 0 }
    }
}
