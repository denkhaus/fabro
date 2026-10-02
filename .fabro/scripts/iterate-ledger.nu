#!/usr/bin/env nu
# Sprint ledger tool (fabro-cadd, ADR-0024 nu-agent iterate model port):
# the ONE deterministic reader/writer of `.fabro/iterate-state.json`.
# Both lanes' closeout and the loop tracker guard call THIS script —
# never parse or hand-edit the state file from another script.
#
# State schema (stable — the session iterate skill reads it too):
#   sprints_completed, sprints_reflected, last_arch_review_at_sprint,
#   last_reflection (ISO date), notes[]
#
# Sprint definition (SETTLED with fabro-cadd, real cadence data sprints
# 1-3): ONE closed seed with a SUBSTANTIVE diff = 1 sprint, ANY lane —
# the provisional fractional loop-weighting alternative (1/3 per loop
# seed) is REJECTED: loop-seed closures are rare and the fraction buys
# complexity without data. Verify-only closures count 0.
#
# Modes (first flag):
#   --mode check          read-only park verdict for the reflection
#                         invariant: parked=true while
#                         sprints_reflected < sprints_completed.
#                         FAIL-OPEN: missing/corrupt state -> parked
#                         false + degraded note (a broken ledger must
#                         not dead-end the line; the tracker guard
#                         adjudicates the residue).
#   --mode close          closeout-side enforcement: --substantive
#                         increments sprints_completed and appends the
#                         sprint note; the line-side short reflection
#                         rides the same close (--reflection), so
#                         sprints_reflected follows to the new count.
#                         Prints {sprints_completed, counted,
#                         gate_due} — gate_due is the arch-gate boundary
#                         signal (sprints_completed % 3 == 0 AND
#                         last_arch_review_at_sprint != count).
#   --mode gate-due       read-only boundary verdict.
#   --mode arch-reviewed  records a completed architecture-gate pass:
#                         last_arch_review_at_sprint = sprints_completed
#                         + --note appended (the LOCAL
#                         improve-codebase-architecture agent's pass is
#                         recorded here; the era constraint keeps the
#                         architect WORKFLOW off the line).
#
# Mutating modes fail loudly (exit 1) so the calling closeout can warn;
# the read modes never exit non-zero on data problems.

const STATE_DEFAULT = (path self | path dirname | path join '..' 'iterate-state.json')

# Parse the state file; null marks missing/corrupt (fail-open classes).
def load-state [at: string]: nothing -> any {
    if not ($at | path exists) { return null }
    let parsed = (try { open --raw $at | from json } catch { null })
    if not ($parsed | describe | str starts-with "record") { return null }
    $parsed
}

def save-state [at: string, state: record]: nothing -> nothing {
    $state | to json | save --force $at
}

# Pure: the reflection-invariant park verdict over a parsed state.
def parked? [state: record]: nothing -> bool {
    (($state.sprints_reflected? | default 0) < ($state.sprints_completed? | default 0))
}

# Pure: the arch-gate boundary verdict over a parsed state.
def gate-due? [state: record]: nothing -> bool {
    let n = ($state.sprints_completed? | default 0)
    ($n > 0) and ($n mod 3 == 0) and (($state.last_arch_review_at_sprint? | default 0) != $n)
}

def main [--mode: string = "check", --state: string, --seed: string = "", --run-id: string = "", --lane: string = "", --reflection: string = "", --substantive, --note: string = ""]: nothing -> nothing {
    let ledger = ($state | default $STATE_DEFAULT)
    let st = (load-state $ledger)

    if $mode == "check" {
        if $st == null {
            {"parked": false, "degraded": true, "reason": "ledger missing or unparsable (fail-open)", "state": $ledger} | to json --raw | print
            return
        }
        {"parked": (parked? $st),
         "degraded": false,
         "sprints_completed": ($st.sprints_completed? | default 0),
         "sprints_reflected": ($st.sprints_reflected? | default 0),
         "state": $ledger} | to json --raw | print
        return
    }

    if $mode == "gate-due" {
        if $st == null {
            {"gate_due": false, "degraded": true, "reason": "ledger missing or unparsable (fail-open)", "state": $ledger} | to json --raw | print
            return
        }
        {"gate_due": (gate-due? $st),
         "sprints_completed": ($st.sprints_completed? | default 0),
         "last_arch_review_at_sprint": ($st.last_arch_review_at_sprint? | default 0),
         "state": $ledger} | to json --raw | print
        return
    }

    if $mode == "close" {
        if $st == null {
            print -e $"iterate-ledger: close refused — state missing/unparsable: ($ledger)"
            exit 1
        }
        if ($reflection | str trim | is-empty) {
            print -e "iterate-ledger: close refused — --reflection is required (line-side short reflection rides every close)"
            exit 1
        }
        mut s = $st
        let counted = $substantive
        let n = (if $counted { ($st.sprints_completed? | default 0) + 1 } else { ($st.sprints_completed? | default 0) })
        let counted_word = (if $counted { "yes" } else { "no — verify-only/substantive-less closure" })
        let sprint_note = ($"sprint ($n) \(seed ($seed), lane ($lane), run ($run_id)\): counted ($counted_word); short reflection — ($reflection)")
        if $counted { $s.sprints_completed = $n }
        $s.sprints_reflected = $n
        $s.last_reflection = (date now | format date "%Y-%m-%d")
        $s.notes = (($s.notes? | default []) | append $sprint_note)
        save-state $ledger $s
        {"outcome": "succeeded",
         "sprints_completed": $n,
         "counted": $counted,
         "gate_due": (gate-due? $s)} | to json --raw | print
        return
    }

    if $mode == "arch-reviewed" {
        if $st == null {
            print -e $"iterate-ledger: arch-reviewed refused — state missing/unparsable: ($ledger)"
            exit 1
        }
        mut s = $st
        let n = ($st.sprints_completed? | default 0)
        $s.last_arch_review_at_sprint = $n
        $s.notes = (($s.notes? | default []) | append $"arch gate pass \(sprint ($n)\): ($note)")
        save-state $ledger $s
        {"outcome": "succeeded", "last_arch_review_at_sprint": $n, "gate_due": false} | to json --raw | print
        return
    }

    print -e $"iterate-ledger: unknown mode '($mode)' \(check|close|gate-due|arch-reviewed\)"
    exit 1
}
