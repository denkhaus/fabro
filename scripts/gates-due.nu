#!/usr/bin/env nu
# Gate arithmetic for the sprint close (born in the sprint-74 retro,
# 2026-11-02: the close hand-computed "no gates due at 75" — wrong, both
# %3 and %5 were due — and only a post-commit self-check caught it. A
# wrong gate statement silently skips reviews; the arithmetic belongs in
# a script, not in a session's head).
#
# The three per-sprint gates (the iterate ledger's own convention,
# pinned by the sprint notes: "no gates due at this close
# (73%2/73%3/73%5 non-zero)"):
#
#   % 2  code review          (last_code_review_at_sprint)
#   % 3  architecture review  (last_arch_review_at_sprint)
#   % 5  rust-security review (last_rust_security_review_at_sprint)
#
# A gate is DUE at a sprint whose number divides evenly, and MISSED if
# its slot came and went without the ledger's last-ran marker moving
# (last_at older than the previous slot). Deterministic, read-only.
# NOTE: prose never sits inside real \(...\) interpolation parens — a
# bare word there parses as a command (the nu lesson, mulched
# 2026-11-02); literal parens are ESCAPED in every string below.
#
#   nu scripts/gates-due.nu            # the NEXT sprint (completed + 1)
#   nu scripts/gates-due.nu 75         # an explicit sprint
#   just gates 75

const GATES = [
    {divisor: 2, name: 'code review', key: 'last_code_review_at_sprint'}
    {divisor: 3, name: 'architecture review', key: 'last_arch_review_at_sprint'}
    {divisor: 5, name: 'rust-security review', key: 'last_rust_security_review_at_sprint'}
]

def fail [msg: string]: nothing -> nothing {
    print -e $"gates-due: ($msg)"
    exit 2
}

def main [sprint?: int] {
    let path = 'iterate-state.json'
    if not ($path | path exists) { fail $"($path) not found — run from the repo root" }
    let state = (open $path)
    let completed = ($state | get -o sprints_completed)
    if ($completed | describe | str starts-with 'int') == false {
        fail $"sprints_completed is not a number: ($completed | describe)"
    }
    let n = (if $sprint != null { $sprint } else { $completed + 1 })
    if $n < 1 { fail $"sprint ($n) is not a positive number" }

    print $"gates for sprint ($n) — ledger sprints_completed = ($completed)"
    mut due = 0
    for gate in $GATES {
        let last_at = ($state | get -o $gate.key | default null)
        let hits = ($n mod $gate.divisor) == 0
        let ran_note = (if $last_at == null { 'never' } else { $last_at | into string })
        if $hits {
            $due = ($due + 1)
            # The previous slot this gate should have run at: missed when
            # the last-ran marker is older than that.
            let prev_slot = ($n - $gate.divisor)
            let missed = (if $last_at == null { $prev_slot >= 1 } else { $last_at < $prev_slot })
            let tail = (if $missed {
                $"MISSED — the slot at sprint ($prev_slot) passed without the ledger recording it"
            } else {
                $"ran previously at sprint ($last_at)"
            })
            print $"  DUE   ($gate.name): \(($n) % ($gate.divisor) == 0\), ($tail)"
        } else {
            print $"  —     ($gate.name) not due: \(($n) % ($gate.divisor) != 0\), ran previously at sprint ($ran_note)"
        }
    }
    if $due == 0 {
        print '  none due — paste this line into the close commit'
    } else {
        print $"  ($due) due — the close commit names every one"
    }
}
