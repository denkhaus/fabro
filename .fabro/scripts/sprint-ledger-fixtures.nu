#!/usr/bin/env nu
# Fixture battery for sprint-ledger.nu (fabro-cadd): proves the nu-agent
# sprint-model port's deterministic core without touching the real
# `.fabro/iterate-state.json` — pure-helper cases plus an end-to-end
# chain against a fabricated ledger in a temp cwd (the script's ledger
# path is cwd-relative by design: workflow scripts run from repo root).
#
# Acceptance mapping (seed fabro-cadd):
#   - counter increment on a substantive close        -> cases 4-6
#   - arch-gate advisory fires on the 3rd boundary    -> case 5
#   - reflection invariant parks unreflected state    -> case 7
#   - reflect closes the gap, park lifts              -> cases 8-9
#   - fail-open on unreadable ledger                  -> cases 2, 10

# Run from the repo root (the qualitygate battery tier does): the path
# is the same relative form the other .fabro/scripts batteries use.
const SCRIPT = 'sprint-ledger.nu'

def fail [what: string]: nothing -> nothing {
    print -e $"sprint-ledger-fixtures: FAIL — ($what)"
    exit 1
}

source $SCRIPT
# The sourced file defines `main`; a script that ends with an in-scope
# `main` AUTO-INVOKES it after the body (the closeout-smoke trap) —
# hide it so the battery's exit is the `print green` below.
hide main

# Absolute form computed BEFORE any cd (the end-to-end cases cd into
# temp cwds; the battery itself is invoked from the repo root).
let ABS = ('.fabro/scripts/sprint-ledger.nu' | path expand)

# --- 1. parse-ledger: valid record parses -----------------------------
let good = (parse-ledger '{"sprints_completed": 3, "sprints_reflected": 3}')
if ($good | describe | str starts-with "record") != true { fail "valid ledger did not parse" }

# --- 2. parse-ledger: garbage / missing fields -> null (fail-open) ----
if (parse-ledger 'not json') != null { fail "garbage parsed non-null" }
if (parse-ledger '{"sprints_reflected": 1}') != null { fail "missing sprints_completed parsed non-null" }

# --- 3. park-decision: proven-unreflected parks, equal does not -------
let parked = (park-decision {sprints_completed: 4, sprints_reflected: 3})
if not $parked.park { fail "unreflected state did not park" }
let ok = (park-decision {sprints_completed: 3, sprints_reflected: 3})
if $ok.park { fail "reflected state parked" }
if (park-decision null).degraded != true { fail "null ledger not degraded" }

# --- 3b. substantive-paths: bookkeeping excluded ----------------------
let subs = (substantive-paths [".seeds/issues.jsonl" ".fabro/journal/x.jsonl" ".fabro/workflows/loop/workflow.fabro" "justfile"])
if $subs != [".fabro/workflows/loop/workflow.fabro" "justfile"] { fail $"substantive filter mismatch: ($subs | to json -r)" }

# --- 4-9. end-to-end chain in a temp cwd ------------------------------
let tmp = (mktemp -d -t "sprint-ledger-fixtures.XXXXXX")
cd $tmp
mkdir .fabro
'{"sprints_completed": 2, "sprints_reflected": 2, "last_arch_review_at_sprint": 0, "notes": []}'
    | save --force .fabro/iterate-state.json

# 4. baseline check: no park (reflected == completed)
let c0 = (do { ^nu $ABS check } | complete | get stdout | from json)
if $c0.park { fail "baseline state parked" }

# 5. record-close: +1 sprint, ARCH GATE DUE advisory at the 3rd boundary
let r1 = (do { ^nu $ABS record-close --seed fabro-fix1 --run-id fixrun } | complete)
let after1 = (open --raw .fabro/iterate-state.json | from json)
if $after1.sprints_completed != 3 { fail $"record-close counted \($after1.sprints_completed), expected 3" }
if ($r1.stderr | str contains "ARCH GATE DUE") != true { fail "arch-gate advisory missing at 3rd boundary" }
if ($after1.notes | length) != 1 { fail "close note not appended" }

# 6. bookkeeping-only diff would count 0 — git failure degrades to
#    counting (documented), so this case pins the pure filter instead:
#    a path list that is ALL bookkeeping is non-substantive.
if (substantive-paths [".seeds/issues.jsonl" ".fabro/journal/a.jsonl"]) != [] { fail "bookkeeping-only diff classified substantive" }

# 7. reflection invariant: check PARKS while unreflected
let c1 = (do { ^nu $ABS check } | complete)
let c1v = ($c1.stdout | from json)
if not $c1v.park { fail "unreflected state after close did not park" }
if ($c1.stderr | str contains "PARK") != true { fail "park warning not printed" }

# 8. reflect closes the gap
let r2 = (do { ^nu $ABS reflect --note "fixture reflection" } | complete)
let after2 = (open --raw .fabro/iterate-state.json | from json)
if $after2.sprints_reflected != 3 { fail "reflect did not set sprints_reflected" }
if ($after2 | get -o last_reflection | default "") == "" { fail "reflect did not stamp last_reflection" }

# 9. park lifts
let c2 = (do { ^nu $ABS check } | complete | get stdout | from json)
if $c2.park { fail "park did not lift after reflect" }

# 10. arch-review-done pins the boundary (no re-advisory next boundary-1)
do { ^nu $ABS arch-review-done } | complete | ignore
let after3 = (open --raw .fabro/iterate-state.json | from json)
if $after3.last_arch_review_at_sprint != 3 { fail "arch-review-done did not pin sprint boundary" }

cd /
rm -rf $tmp

# --- 11. missing ledger: check fails open, never parks -----------------
let tmp2 = (mktemp -d -t "sprint-ledger-empty.XXXXXX")
cd $tmp2
let c3 = (do { ^nu $ABS check } | complete)
if $c3.exit_code != 0 { fail "check failed on missing ledger (must fail-open)" }
let c3v = ($c3.stdout | from json)
if $c3v.park or (not $c3v.degraded) { fail "missing ledger must report park=false degraded=true" }
cd /
rm -rf $tmp2

print "sprint-ledger fixtures green"
