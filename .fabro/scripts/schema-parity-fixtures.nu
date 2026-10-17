#!/usr/bin/env nu
# Planner-output schema parity battery (fabro-899b, arch-gate sprint 42
# candidate 2): .fabro/workflows/develop/schemas/planner-output.schema.json
# and .fabro/workflows/loop/schemas/planner-output.schema.json are the last
# measured live copy pair of the fabro-8922 family (0.913-similar) — the
# complete delta is the LANE AXIS, and a schema edit on one side silently
# stops matching the other. This battery parses both and asserts
# loop == develop modulo an explicit LANE_AXIS allow-list:
#   - root `title` and `description` (lane prose, deliberately distinct)
#   - the gate-ban node's `description`
#   - the gate-ban `pattern`: loop's negative-lookahead ban set must be a
#     SUPerset of develop's (loop bans both gates; develop bans the product
#     gate only). A develop-side ban with no loop twin REDs until mirrored.
# Any other divergence — new key, enum edit, required-key change, ban-shell
# drift — REDs. Cross-workflow file sharing is NOT the fix (bundle-root
# limits, fabro-aeb7); each schema stays its lane's engine-validated
# output_schema contract.
#
# Battery contract: pure-logic, no cargo, no live server (battery-runner
# tier). Teeth run against temp fixtures under mktemp -d OUTSIDE the
# worktree, through this same script's `check` subcommand.

const DEVELOP_SCHEMA = '.fabro/workflows/develop/schemas/planner-output.schema.json'
const LOOP_SCHEMA = '.fabro/workflows/loop/schemas/planner-output.schema.json'
const BAN_PATH = '.properties.context_updates.properties.current_seed_brief.pattern'

# Lane-axis paths whose drift is allowed (everything else REDs).
const LANE_AXIS = [
    '.title'
    '.description'
    '.properties.context_updates.properties.current_seed_brief.description'
    $BAN_PATH
]

# Recursive structural diff: every divergence as '<path>: <what>'. Paths
# are dotted JSON paths with a leading '.' at the root.
def deep-diff [a, b, base: string]: nothing -> list<string> {
    let ta = ($a | describe)
    let tb = ($b | describe)
    if $ta != $tb {
        [$"($base): type drift develop=($ta) loop=($tb)"]
    } else if ($ta | str starts-with 'record') {
        let ka = ($a | columns)
        let kb = ($b | columns)
        mut errs = []
        for k in ($ka | where {|x| $x not-in $kb}) {
            $errs = ($errs | append $"($base).($k): key only in develop schema")
        }
        for k in ($kb | where {|x| $x not-in $ka}) {
            $errs = ($errs | append $"($base).($k): key only in loop schema")
        }
        for k in ($ka | where {|x| $x in $kb}) {
            $errs = ($errs | append (deep-diff ($a | get $k) ($b | get $k) $"($base).($k)"))
        }
        $errs
    } else if ($ta | str starts-with 'list') {
        if ($a | length) != ($b | length) {
            [$"($base): length drift develop=($a | length) loop=($b | length)"]
        } else if ($a | length) > 0 {
            mut errs = []
            for i in 0..(($a | length) - 1) {
                $errs = ($errs | append (deep-diff ($a | get $i) ($b | get $i) $"($base)[($i)]"))
            }
            $errs
        } else {
            []
        }
    } else if $a == $b {
        []
    } else {
        [$"($base): value drift develop=($a | to json -r) loop=($b | to json -r)"]
    }
}

# The negative-lookahead ban alternatives of a gate-ban pattern, or an
# empty list when the pattern carries no `(?!...)` group.
def parse-bans [pattern: string]: nothing -> list<string> {
    let m = ($pattern | parse --regex '\(\?!(?P<bans>[^)]*)\)')
    if ($m | is-empty) { [] } else { $m.bans.0 | split row '|' }
}

# Gate-ban axis check: both patterns must sit in the canonical
# `^(?:(?!<bans>)[\s\S])*$` shell, and loop's ban set must contain every
# develop ban (loop adds its own gate on top — the strict superset).
def ban-errors [develop_pattern: string, loop_pattern: string]: nothing -> list<string> {
    let db = (parse-bans $develop_pattern)
    let lb = (parse-bans $loop_pattern)
    let canonical = {|bans|
        let body = ($bans | str join '|')
        '^(?:(?!' ++ $body ++ ')[\s\S])*$'
    }
    mut errs = []
    if $develop_pattern != (do $canonical $db) {
        $errs = ($errs | append ($BAN_PATH ++ ': develop gate-ban pattern is not the canonical `^(?:(?!<bans>)[\s\S])*$` shell'))
    }
    if $loop_pattern != (do $canonical $lb) {
        $errs = ($errs | append ($BAN_PATH ++ ': loop gate-ban pattern is not the canonical `^(?:(?!<bans>)[\s\S])*$` shell'))
    }
    for b in $db {
        if $b not-in $lb {
            $errs = ($errs | append $"($BAN_PATH): develop gate-ban alternative '($b)' missing from loop's ban set — mirror it in the SAME change")
        }
    }
    if ($db | is-not-empty) and ($lb | where {|x| $x not-in $db} | is-empty) {
        $errs = ($errs | append $"($BAN_PATH): loop gate-ban pattern bans nothing beyond develop — expected the loop gate alternative on top")
    }
    $errs
}

# Full parity verdict for one schema pair (records already parsed).
def parity-errors [d: record, l: record]: nothing -> list<string> {
    let raw = (deep-diff $d $l '')
    let hard = ($raw | where {|e| ($e | split row ':' | first) not-in $LANE_AXIS })
    let dpat = ($d | get -o properties.context_updates.properties.current_seed_brief.pattern | default '')
    let lpat = ($l | get -o properties.context_updates.properties.current_seed_brief.pattern | default '')
    if ($dpat | is-empty) or ($lpat | is-empty) {
        $hard | append $"($BAN_PATH): gate-ban pattern missing on one side — the parity contract assumes it exists in both schemas"
    } else {
        $hard | append (ban-errors $dpat $lpat)
    }
}

# One-shot check over two schema file paths: exit 0 green, exit 1 RED
# naming every divergence. The teeth fixtures and any manual audit run
# through THIS entry point, never a private copy of the logic.
def "main check" [develop: string, loop: string]: nothing -> nothing {
    let errs = (parity-errors (open $develop) (open $loop))
    if ($errs | is-not-empty) {
        print -e $"schema-parity: develop=($develop) loop=($loop)"
        for e in $errs { print -e $"schema-parity: ($e)" }
        exit 1
    }
    print 'schema-parity: ok'
}

def expect-red [name: string, develop: string, loop: string]: nothing -> bool {
    let res = (do { ^nu $env.CURRENT_FILE check $develop $loop } | complete)
    if $res.exit_code == 1 {
        print $"schema-parity teeth: ($name) RED as required"
        true
    } else {
        print -e $"schema-parity teeth: ($name) was expected RED but exited ($res.exit_code)"
        false
    }
}

def main []: nothing -> nothing {
    # 1. The live pair is the pin: green here means the lane axis is the
    #    COMPLETE delta between the two schemas.
    main check $DEVELOP_SCHEMA $LOOP_SCHEMA

    # 2. Teeth (fabro-899b): every divergence class REDs. Fixtures are the
    #    real schemas mutated under mktemp -d OUTSIDE the worktree — the
    #    check logic under test is this file's own `check` subcommand.
    let tmp = (mktemp -d -t schema-parity.XXXXXX)
    let dpath = $"($tmp)/develop.schema.json"
    let lpath = $"($tmp)/loop.schema.json"
    let d = (open $DEVELOP_SCHEMA)
    let l = (open $LOOP_SCHEMA)

    # green control: the untouched pair passes through the temp copy too
    $d | to json | save $dpath
    $l | to json | save $lpath
    let ctl = (do { ^nu $env.CURRENT_FILE check $dpath $lpath } | complete)
    if $ctl.exit_code != 0 {
        print -e $"schema-parity teeth: control pair was expected green but exited ($ctl.exit_code)"
        print -e ($ctl.stderr | str trim -r -c "\n")
        exit 1
    }
    print 'schema-parity teeth: untouched pair green'

    mut green = true
    # extra key on the loop side
    ($l | insert 'x-parity-teeth' true | to json) | save -f $lpath
    $green = ($green and (expect-red 'extra root key (loop side)' $dpath $lpath))
    # enum edit on the loop side
    ($l | update properties.preferred_next_label.enum.4 'Parked' | to json) | save -f $lpath
    $green = ($green and (expect-red 'preferred_next_label enum drift' $dpath $lpath))
    # required-key addition on the develop side
    ($d | update required {|r| $r.required | append 'x-parity-teeth'} | to json) | save -f $dpath
    $green = ($green and (expect-red 'required-key drift (develop side)' $dpath $lpath))
    # restore the clean develop fixture before the later teeth (fabro-2366):
    # the required-key tooth used to leave 'x-parity-teeth' in $dpath, so
    # every later tooth RED'd on the leftover drift too instead of isolating
    # its own divergence class.
    $d | to json | save -f $dpath
    # a develop ban alternative missing from loop's set
    ($l | update properties.context_updates.properties.current_seed_brief.pattern {(| '^(?:(?!just qualitygate|nu\s+\.fabro/workflows/loop/scripts/loop-gate\.nu)[\s\S])*$' )} | to json) | save -f $lpath
    $green = ($green and (expect-red 'develop ban alternative missing from loop' $dpath $lpath))
    # ban-shell drift (pattern escapes the canonical shell)
    ($l | update properties.context_updates.properties.current_seed_brief.pattern 'just qualitygate' | to json) | save -f $lpath
    $green = ($green and (expect-red 'gate-ban pattern shell drift' $dpath $lpath))
    # loop bans nothing beyond develop (drops the loop-gate alternative)
    ($l | update properties.context_updates.properties.current_seed_brief.pattern {(| '^(?:(?!just qualitygate|nu\s+scripts/qualitygate\.nu)[\s\S])*$' )} | to json) | save -f $lpath
    $green = ($green and (expect-red 'loop gate-ban set not a strict superset' $dpath $lpath))

    rm -rf $tmp
    if not $green { exit 1 }
    print 'schema-parity: ok — develop/loop planner-output schemas pinned modulo the lane axis'
}
