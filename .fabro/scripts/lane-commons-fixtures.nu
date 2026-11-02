#!/usr/bin/env nu
# Lane-script commons battery (denkhaus/work#1108): the work-channel and
# run-anchor modules exist so the MIRRORED closeout/evidence helpers
# ("reuse this shape, do not invent a new one" comments) become ONE
# implementation. Two pin families:
#
#   1. PURE pins — the branch parse (anchored end, off-run default
#      parametrized per consumer), the fixed-string grep mark (the
#      escape incident: a regex-shaped mark matched nothing), and the
#      argv builders (the --project-after-subcommand lesson of run
#      01M60NX9W7N65DEPXYN09CE9BR, the label join).
#   2. EXTRACTION pins — every consumer SOURCES the modules (the `use`
#      lines exist) and NO mirror regrows (no local work-call/run-base/
#      current-branch defs in the consumers). The extraction pins are
#      the red-first core of #1108: red before the extraction (helpers
#      still mirrored), green after.
#
# The transport arms (server refusal, end-to-end argv against a channel)
# stay pinned by the closeout/evidence smokes, which drive the real
# scripts end to end.

const DIR = (path self | path dirname)

use lib/run_anchor.nu
use lib/work_channel.nu

# The consumers whose mirrors retired into the lib (#1108): the develop
# graph's stage scripts. check-transcript only consumed the run-anchor
# triple; closeout and evidence consumed both modules.
const CONSUMERS_BOTH = [
    'closeout.nu'
    'evidence.nu'
]
const CONSUMER_ANCHOR = [
    'check-transcript.nu'
]

def consumer-src [name: string]: nothing -> string {
    open --raw ($DIR | path join '..' 'workflows' 'develop' 'scripts' $name)
}

def assert-eq [got: any, want: any, what: string]: nothing -> nothing {
    if $got != $want {
        print -e $"lane-commons: ($what): got ($got | to json --raw), want ($want | to json --raw)"
        exit 1
    }
}

def assert-contains [haystack: string, needle: string, what: string]: nothing -> nothing {
    if not ($haystack | str contains $needle) {
        print -e $"lane-commons: ($what): missing `($needle)`"
        exit 1
    }
}

# First index of a value in a list, -1 when absent.
def first-index-of [list: list<string>, value: string]: nothing -> int {
    let hit = ($list | enumerate | where item == $value | get -o index | default [])
    if ($hit | is-empty) { -1 } else { $hit | first }
}

# --- 1. pure pins: the branch parse ---------------------------------------

def test-id-from-branch []: nothing -> nothing {
    # A run branch yields the id — exact, no fragment of it.
    assert-eq (run_anchor id-from-branch 'fabro/run/01M5844J1FDD') '01M5844J1FDD' 'run branch yields the id'
    # Off a run branch: the CONSUMER's default, both shapes in use.
    assert-eq (run_anchor id-from-branch 'main') '' 'off-run default is empty string by default'
    assert-eq (run_anchor id-from-branch 'main' 'local') 'local' 'off-run default parametrized (evidence reads local)'
    # The regex anchors the branch END: a nested path is not a run branch.
    assert-eq (run_anchor id-from-branch 'fabro/run/a/b') '' 'nested path under fabro/run is off-run'
    # A detached HEAD passes '' as the branch: off-run, never a crash.
    assert-eq (run_anchor id-from-branch '') '' 'empty branch (detached HEAD) is off-run'
}

def test-subject-mark []: nothing -> nothing {
    # The mark carries LITERAL parentheses for --fixed-strings: the
    # value has no backslashes (the source's `\(` is string-escaping
    # against interpolation, not part of the value). A regex-shaped
    # mark like `fabro.<id>:` silently matched nothing and emptied the
    # evidence sections.
    assert-eq (run_anchor subject-mark '01M5844J1FDD') 'fabro(01M5844J1FDD):' 'subject mark carries literal parentheses for --fixed-strings'
}

# --- 2. pure pins: the work-channel argv builders --------------------------

def test-work-create-argv []: nothing -> nothing {
    let argv = (work_channel create-argv 'https://mirtuell.net' 'A title' 'task' ['residual' 'lane:loop'])
    # Facade first, server flag at the facade level, SUBCOMMAND before
    # --project: a pre-subcommand --project makes clap eat the verb as
    # --server's value (run 01M60NX9W7N65DEPXYN09CE9BR, the
    # deferred-action loss — the shim is position-blind, so this pin
    # holds the shape against the real parser).
    assert-eq ($argv | first) 'work' 'create argv starts at the work facade'
    assert-eq ($argv | any {|a| $a == '--server'}) true 'create argv carries --server at the facade level'
    let create_at = (first-index-of $argv 'create')
    let project_at = (first-index-of $argv '--project')
    if $create_at == -1 { print -e 'lane-commons: create argv lost the create subcommand'; exit 1 }
    if $project_at <= $create_at {
        print -e $"lane-commons: --project at ($project_at) must come AFTER the create subcommand at ($create_at)"
        exit 1
    }
    assert-eq ($argv | any {|a| $a == '--body-file'}) true 'create argv carries --body-file (tracker text never on a command line)'
    # Labels ride ONE comma-joined --label value (flat argv, the CLI
    # accepts exactly this form).
    assert-eq ($argv | any {|a| $a == 'residual,lane:loop'}) true 'labels join into one comma-joined --label value'
}

def test-observation-record-argv []: nothing -> nothing {
    let argv = (work_channel observation-record-argv 'https://mirtuell.net' 'painpoint' 'a fact')
    assert-eq ($argv | first) 'observation' 'observation argv starts at the observation facade'
    let record_at = (first-index-of $argv 'record')
    let category_at = (first-index-of $argv '--category')
    if $record_at == -1 { print -e 'lane-commons: observation argv lost the record subcommand'; exit 1 }
    if $category_at <= $record_at {
        print -e $"lane-commons: --category at ($category_at) must come AFTER the record subcommand at ($record_at)"
        exit 1
    }
    assert-eq ($argv | any {|a| $a == '--text'}) true 'observation argv carries --text'
}

# --- 2b. probe-literal equality: the lib argv IS the bench literal --------
# The bench registry (the fabro test repo) pins the argv GRAMMAR against the toolchain's
# fabro CLI with hand-spelled literals (probe-28: work-create; probe-37:
# observation record). The composition hole without this arm: a drifting
# lib argv would leave both sides green (fixtures pin the lib's own
# output, the probes pin their literals). Equality closes it — lib argv
# == bench literal == toolchain-clap-parsed shape.

def test-probe-literal-equality []: nothing -> nothing {
    # probe-28's literal (workbench.nu probe-28-work-create-argv), argv
    # after the binary, verbatim.
    let probe28 = [
        'work' '--server' 'http://127.0.0.1:1' 'create' '--project' 'fabro'
        '--title' 'pin' '--body-file' '-' '--type' 'task' '--label' 'pin'
    ]
    assert-eq (work_channel create-argv 'http://127.0.0.1:1' 'pin' 'task' ['pin']) $probe28 'create-argv equals the probe-28 bench literal'

    # probe-37's literal (its probe.nu observation call), argv after the
    # binary, verbatim.
    let probe37 = [
        'observation' '--server' 'http://127.0.0.1:32276' 'record'
        '--category' 'note' '--text' 'probe-37 recorded from a script stage'
    ]
    assert-eq (work_channel observation-record-argv 'http://127.0.0.1:32276' 'note' 'probe-37 recorded from a script stage') $probe37 'observation-record-argv equals the probe-37 bench literal'

    # probe-39's literals (its probe.nu door calls — the adjudicator's
    # four READ doors), argv after the binary, verbatim. The server
    # value is the workbench default the live sock stands in for (the
    # probe-37 equality convention); the run id and item key are the
    # representative literals of the probe's grammar legs.
    let probe39_queue = [
        'run' 'queue' '--field' 'merge' '--workflow' 'develop' '--json'
        '--server' 'http://127.0.0.1:32276'
    ]
    assert-eq (work_channel run-queue-argv 'http://127.0.0.1:32276' 'merge' 'develop') $probe39_queue 'run-queue-argv equals the probe-39 bench literal'

    let probe39_facts = [
        'observation' '--server' 'http://127.0.0.1:32276' 'list'
        '--run-id' '01M5844J1FDD00000000000000' '--json'
    ]
    assert-eq (work_channel observation-list-argv 'http://127.0.0.1:32276' '01M5844J1FDD00000000000000') $probe39_facts 'observation-list-argv equals the probe-39 bench literal'

    let probe39_radius = [
        'work' '--server' 'http://127.0.0.1:32276' 'radius' '--project'
        'fabro-proof' 'denkhaus/work#7' '--json'
    ]
    assert-eq (work_channel work-radius-argv 'http://127.0.0.1:32276' 'fabro-proof' 'denkhaus/work#7') $probe39_radius 'work-radius-argv equals the probe-39 bench literal'

    let probe39_comments = [
        'work' '--server' 'http://127.0.0.1:32276' 'comments' '--project'
        'fabro-proof' 'denkhaus/work#7' '--json'
    ]
    assert-eq (work_channel work-comments-argv 'http://127.0.0.1:32276' 'fabro-proof' 'denkhaus/work#7') $probe39_comments 'work-comments-argv equals the probe-39 bench literal'
}

# --- 3. extraction pins: the mirrors retired -------------------------------

def test-consumers-source-the-lib []: nothing -> nothing {
    for name in $CONSUMERS_BOTH {
        let text = (consumer-src $name)
        assert-contains $text 'use ../../../scripts/lib/work_channel.nu' $"($name) sources the work-channel module"
        assert-contains $text 'use ../../../scripts/lib/run_anchor.nu' $"($name) sources the run-anchor module"
    }
    for name in $CONSUMER_ANCHOR {
        let text = (consumer-src $name)
        assert-contains $text 'use ../../../scripts/lib/run_anchor.nu' $"($name) sources the run-anchor module"
    }
}

def test-no-mirror-regrows []: nothing -> nothing {
    # The mirror markers: any of these defs back in a consumer means a
    # copy regrew beside the lib (the state #1108 retired).
    let mirrors = ['def work-call' 'def work-server' 'def run-base' 'def current-branch' 'def current-run-id' 'def work-create']
    let names = ($CONSUMERS_BOTH | append $CONSUMER_ANCHOR)
    for name in $names {
        let text = (consumer-src $name)
        for marker in $mirrors {
            if ($text | str contains $marker) {
                print -e $"lane-commons: mirror regrew in ($name): `($marker)` — the lib owns that shape now"
                exit 1
            }
        }
    }
}

def main []: nothing -> nothing {
    test-id-from-branch
    test-subject-mark
    test-work-create-argv
    test-observation-record-argv
    test-probe-literal-equality
    test-consumers-source-the-lib
    test-no-mirror-regrows
    print 'lane-commons: all pins green'
}
