#!/usr/bin/env nu
# Fixture battery for .fabro/scripts/iterate-ledger.nu (fabro-cadd): the
# deterministic acceptance tests for the sprint-ledger port — counter
# updates on close (substantive vs verify-only), the arch-gate boundary
# (fires at sprint 3 and 6, cleared by arch-reviewed, never off-boundary),
# and the reflection-invariant park (blocks a simulated unreflected
# state). Runs against a THROWAWAY state file in a temp dir — the real
# `.fabro/iterate-state.json` is never touched. Wired into
# scripts/qualitygate.nu's loop-asset tier (dup-run-check-fixtures
# pattern):
#   nu .fabro/scripts/iterate-ledger-smoke.nu

const LEDGER = (path self | path dirname | path join 'iterate-ledger.nu')
source $LEDGER

def fail [what: string]: nothing -> nothing {
    print -e $"iterate-ledger-smoke: FAIL — ($what)"
    exit 1
}

def call [args: list<string>]: nothing -> any {
    let res = (do { nu $LEDGER ...$args } | complete)
    {res: $res, json: (try { $res.stdout | str trim | from json } catch { null })}
}

def fresh-state [dir: string, n: int]: nothing -> nothing {
    {sprints_completed: $n, sprints_reflected: $n, last_arch_review_at_sprint: 0, last_reflection: "", notes: []}
    | to json
    | save --force ($dir | path join $"st($n).json")
}

# --- Pure verdicts over parsed states ---
if not (parked? {sprints_completed: 3, sprints_reflected: 2}) { fail "parked? 3/2 must be true" }
if (parked? {sprints_completed: 3, sprints_reflected: 3}) { fail "parked? 3/3 must be false" }
if not (gate-due? {sprints_completed: 3, last_arch_review_at_sprint: 0}) { fail "gate-due? sprint 3 unreviewed must be true" }
if (gate-due? {sprints_completed: 3, last_arch_review_at_sprint: 3}) { fail "gate-due? reviewed sprint 3 must be false" }
if (gate-due? {sprints_completed: 4, last_arch_review_at_sprint: 3}) { fail "gate-due? off-boundary sprint 4 must be false" }
if (gate-due? {sprints_completed: 0}) { fail "gate-due? sprint 0 must be false" }

let dir = (mktemp -d)

# --- check: fail-open on missing state, park on unreflected state ---
let ck_missing = (call [--mode check --state $"($dir | path join 'nope.json')"])
if $ck_missing.res.exit_code != 0 { fail "check on missing state must exit 0" }
if ($ck_missing.json?.parked? | default true) != false { fail "check on missing state must not park (fail-open)" }
if ($ck_missing.json?.degraded? | default false) != true { fail "check on missing state must be degraded" }

fresh-state $dir 2
let st2 = ($dir | path join 'st2.json')
let ck_ok = (call [--mode check --state $st2])
if ($ck_ok.json?.parked? | default true) != false { fail "check on reflected state must not park" }

# --- close: substantive increments and rides the reflection ---
let cl = (call [--mode close --state $st2 --seed fabro-test1 --run-id run-test1 --lane loop --substantive --reflection 'smoke reflection one'])
if $cl.res.exit_code != 0 { fail $"close substantive failed: ($cl.res.stderr)" }
if ($cl.json?.sprints_completed? | default (-1)) != 3 { fail $"close substantive count != 3: ($cl.res.stdout)" }
if ($cl.json?.counted? | default false) != true { fail "close substantive counted flag missing" }
# boundary: sprint 3, unreviewed -> gate_due FIRES (the deterministic
# acceptance criterion: the gate fires on the 3rd sprint boundary).
if ($cl.json?.gate_due? | default false) != true { fail "close at sprint 3 must fire gate_due" }
# reflection rode the close: not parked after.
let ck3 = (call [--mode check --state $st2])
if ($ck3.json?.parked? | default true) != false { fail "close must clear the reflection park" }

# --- arch-reviewed clears the boundary; next boundary is 6 ---
let ar = (call [--mode arch-reviewed --state $st2 --note 'smoke arch pass'])
if $ar.res.exit_code != 0 { fail "arch-reviewed failed" }
if ($ar.json?.gate_due? | default true) != false { fail "arch-reviewed must clear gate_due" }
# sprint 4: not a boundary.
let cl4 = (call [--mode close --state $st2 --seed fabro-test2 --run-id run-test2 --lane develop --substantive --reflection 'smoke reflection two'])
if ($cl4.json?.gate_due? | default true) != false { fail "sprint 4 must not fire gate_due" }
# sprint 5: not a boundary.
let cl5 = (call [--mode close --state $st2 --seed fabro-test3 --run-id run-test3 --lane develop --substantive --reflection 'smoke reflection three'])
if ($cl5.json?.gate_due? | default true) != false { fail "sprint 5 must not fire gate_due" }
# sprint 6: boundary again.
let cl6 = (call [--mode close --state $st2 --seed fabro-test4 --run-id run-test4 --lane loop --substantive --reflection 'smoke reflection four'])
if ($cl6.json?.sprints_completed? | default (-1)) != 6 { fail "sprint 6 count wrong" }
if ($cl6.json?.gate_due? | default false) != true { fail "sprint 6 must fire gate_due" }

# --- close: verify-only counts 0 ---
fresh-state $dir 3
let st3 = ($dir | path join 'st3.json')
let clv = (call [--mode close --state $st3 --seed fabro-verify --run-id run-verify --lane loop --reflection 'verify-only closure'])
if ($clv.json?.sprints_completed? | default (-1)) != 3 { fail "verify-only close must not increment" }
if ($clv.json?.counted? | default true) != false { fail "verify-only counted flag must be false" }

# --- close: reflection is REQUIRED (fail loud) ---
let clnorefl = (call [--mode close --state $st3 --seed fabro-x --run-id run-x --lane loop --substantive])
if $clnorefl.res.exit_code == 0 { fail "close without --reflection must fail" }

# --- check: park on a simulated unreflected state ---
{sprints_completed: 4, sprints_reflected: 3, last_arch_review_at_sprint: 3, last_reflection: "", notes: []} | to json | save --force ($dir | path join 'park.json')
let ck_park = (call [--mode check --state $"($dir | path join 'park.json')"])
if ($ck_park.json?.parked? | default false) != true { fail "simulated unreflected state must park" }
rm -rf $dir

print "iterate-ledger-smoke: ok — counters, arch-gate boundary (3 and 6), reflection invariant, fail-open contracts verified"

# Sourcing imports def main; exit explicitly (closeout-smoke idiom).
exit 0
