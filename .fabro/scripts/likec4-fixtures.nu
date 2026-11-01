#!/usr/bin/env nu
# likec4 landscape battery (retro 2026-11-01): the .likec4/ architecture
# landscape must VALIDATE — spec kinds, model FQNs, view predicates, the
# deployment topology. A nonexistent shape token once reached the user
# before any check ran; the sprint-close rule carried the validate call
# as prose only. This battery is the mechanical net: every battery sweep
# validates the project and REDs on any diagnostic.
#
# HOST-ONLY, loudly skipped where it cannot run: the check shells out to
# bunx against .likec4/node_modules, which is gitignored. Where bun or
# the installed toolchain is absent — run sandboxes — the battery
# announces a skip and stays green, so the loop gate never REDs on a
# missing dev toolchain. The skip is a capability statement, never a
# hiding place: the host sweep, qualitygate `all` on every dev host, is
# the net this battery exists for.
#
#   nu .fabro/scripts/likec4-fixtures.nu

const SCRIPT_DIR = (path self | path dirname)
const ROOT = ($SCRIPT_DIR | path join '../..' | path expand)
const LIKEC4 = ($ROOT | path join '.likec4')

def fail [what: string]: nothing -> nothing {
    print -e $"likec4: FAIL — ($what)"
    exit 1
}

if not ($LIKEC4 | path exists) {
    fail ".likec4/ does not exist — the landscape is tracked; a missing directory is a finding, not a skip"
}

let has_bun = ((which bunx | length) > 0)
let has_deps = ($LIKEC4 | path join 'node_modules' | path exists)
if (not $has_bun) or (not $has_deps) {
    print $"likec4: host-only skip — bunx found: ($has_bun), node_modules installed: ($has_deps); validate on a dev host after 'bun install' in .likec4/"
    exit 0
}

let res = (do { cd $LIKEC4; ^bunx likec4 validate --json --no-layout . } | complete)
if $res.exit_code != 0 {
    # The runner exits nonzero on diagnostics: surface them verbatim so
    # the RED names file and line, not just the exit.
    print -e ($res.stdout | str trim)
    print -e ($res.stderr | str trim)
    fail "likec4 validate reported diagnostics — fix the landscape files above"
}

let report = ($res.stdout | from json)
if ($report | get -o valid) == false {
    for e in ($report | get -o errors | default []) {
        let file = ($e | get -o file | default '?')
        let line = ($e | get -o line | default '?')
        let msg = ($e | get -o message | default '?')
        print -e $"likec4: ($file):($line) — ($msg | str substring 0..160)"
    }
    fail "the landscape has validation errors"
}

let files = ($report | get -o stats | default {} | get -o totalFiles | default '?')
print $"likec4: green — landscape validates, ($files) source files"
