#!/usr/bin/env nu
# Sprint close, ONE entry point (born in the sprint-74 retro, 2026-11-02:
# the close was hand-driven JSON surgery — the rotation script existed
# but was bypassed because its pointer lived only in a justfile comment,
# the archive convention was reconstructed from git log, and the gate
# arithmetic went wrong in the commit message. Every step below already
# existed somewhere; this script makes them one command).
#
#   nu scripts/sprint-close.nu <sprint> [flags]   # the note rides stdin
#
# Steps, in order:
#   1. validate: sprint == sprints_completed + 1 (no gaps, no re-close;
#      --force overrides for repairs, replacing nothing — it still
#      refuses, repairs are hand work).
#   2. the sprint note on stdin, appended as
#      {sprint: N, notes: ["…"]}; sprints_completed = N; every
#      --ran-* flag moves its last_*_at_sprint marker to N (a gate that
#      RAN at this close records that it ran).
#   3. rotation via scripts/rotate-sprint-notes.nu (keep 10, archive
#      JSONL) — never hand-roll this again.
#   4. gates for sprint N+1 via scripts/gates-due.nu, plus the
#      commit-message skeleton with the gate line filled — paste both.
#
# The state file is written with 1-space indent (the format the ledger
# has carried since its python-era edits; nu's `to json --indent 1`
# matches it byte for byte).

# Parse-time self path (the up.nu precedent: `path self` errors at run
# time since nu 0.115) — sibling scripts resolve from here, not CWD.
const SCRIPTS_DIR = (path self | path dirname)

def fail [msg: string]: nothing -> nothing {
    print -e $"sprint-close: ($msg)"
    exit 2
}

def main [
    sprint: int,
    --ran-code-review,   # the %2 code review ran at this close
    --ran-arch-review,   # the %3 architecture review ran at this close
    --ran-rust-security, # the %5 rust-security review ran at this close
] {
    let state_path = 'iterate-state.json'
    if not ($state_path | path exists) { fail $"($state_path) not found — run from the repo root" }
    let state = (open $state_path)
    let completed = ($state | get -o sprints_completed)
    if ($completed | describe | str starts-with 'int') == false {
        fail $"sprints_completed is not a number: ($completed | describe)"
    }
    if $sprint != ($completed + 1) {
        fail $"sprint ($sprint) does not follow sprints_completed=($completed) — a re-close or a gap is hand work, not this script"
    }

    # Piped stdin, the work-shim's proven shape: `cat | str join`.
    let note = (cat | str join | str trim)
    if ($note | is-empty) { fail 'the sprint note is empty — pipe it on stdin' }

    # 1+2: append the record, move the completed pointer and the ran
    # markers. upsert keeps the record's key order stable.
    let record = {sprint: $sprint, notes: [$note]}
    let next = ($state
        | upsert sprints ($state.sprints | append $record)
        | upsert sprints_completed $sprint)
    let next = (if $ran_code_review {
        $next | upsert last_code_review_at_sprint $sprint
    } else { $next })
    let next = (if $ran_arch_review {
        $next | upsert last_arch_review_at_sprint $sprint
    } else { $next })
    let next = (if $ran_rust_security {
        $next | upsert last_rust_security_review_at_sprint $sprint
    } else { $next })
    $next | to json --indent 1 | save --force $state_path
    let markers = (if ($ran_code_review or $ran_arch_review or $ran_rust_security) {
        ', gate markers moved'
    } else {
        ''
    })
    print $"sprint-close: record ($sprint) appended, sprints_completed=($sprint)($markers)"

    # 3: rotation (idempotent, archives overflow to the JSONL)
    nu ($SCRIPTS_DIR | path join 'rotate-sprint-notes.nu')

    # 4: gates for the NEXT sprint + the commit skeleton
    print ''
    nu ($SCRIPTS_DIR | path join 'gates-due.nu') ($sprint + 1)
    print ''
    print '--- commit skeleton ---'
    print $"sprint ledger: sprint ($sprint) closed \(rotate the subject; name the gates that ran)"
    print ''
    print 'Sprint <N> record: <one line on what landed and what the record carries>.'
    print 'Gates for the next close: <paste the gates-due block above>.'
}
