#!/usr/bin/env nu
# Lane-script commons battery (denkhaus/work#1108): the work-channel and
# run-anchor modules exist so the MIRRORED reporter/evidence helpers
# ("reuse this shape, do not invent a new one" comments) become ONE
# implementation. Pin families:
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
#   3. NO-INLINE-ARGV pins (work#1150) — the def-only granularity hole:
#      the mirror scan matched local `def`s, so an inline argv literal
#      passed green — exactly how the lane-check work-list and the
#      stall-check automations/ps literals landed. Every `--server`
#      token on a consumer's code line is a re-grown literal now.
#   4. QUOTE-GUARD pins (work#1190) — the recorded-check wrapper refuses
#      the nested-shell metachar trap (a `sh -c` payload parses twice;
#      run 01M6X3WCQ1's unquoted graphviz arrow became the stray
#      repo-root file `b` in the evidence commit). The boundary pins
#      hold the detector's refusals AND its allowed forms.
#
# The transport arms (server refusal, end-to-end argv against a channel)
# stay pinned by the reporter/evidence smokes, which drive the real
# scripts end to end.

const DIR = (path self | path dirname)

use lib/run_anchor.nu
use lib/work_channel.nu
use lib/terminal_status.nu [TERMINAL_KINDS, is-terminal-kind, is-success-kind, gate-classification, HUMAN_NEEDED_KIND, is-human-needed-kind]
use ../workflows/develop/scripts/check-transcript.nu

# The consumers whose mirrors retired into the lib (#1108): the develop
# graph's stage scripts. check-transcript only consumed the run-anchor
# triple; reporter (the stage-G facts stage, closeout's successor) and
# evidence consumed both modules.
const CONSUMERS_BOTH = [
    'reporter.nu'
    'evidence.nu'
]
const CONSUMER_ANCHOR = [
    'check-transcript.nu'
]
# work#1150: the two door consumers whose inline argv retired into the
# builders — scanned by the no-inline-argv check beside the develop
# consumers.
const CONSUMER_LANES = [
    [workflow name];
    [develop lane-check.nu]
    [watchdog stall-check.nu]
]

def consumer-src [workflow: string, name: string]: nothing -> string {
    open --raw ($DIR | path join '..' 'workflows' $workflow 'scripts' $name)
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

def assert-true [got: bool, what: string]: nothing -> nothing {
    if not $got {
        print -e $"lane-commons: ($what): expected true, got false"
        exit 1
    }
}

def assert-false [got: bool, what: string]: nothing -> nothing {
    if $got {
        print -e $"lane-commons: ($what): expected false, got true"
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

def test-run-set-title-argv []: nothing -> nothing {
    let argv = (work_channel run-set-title-argv 'https://mirtuell.net' 'Watchdog: all enabled gears healthy')
    assert-eq ($argv | first) 'run' 'set-title argv starts at the run facade'
    let verb_at = (first-index-of $argv 'set-title')
    let server_at = (first-index-of $argv '--server')
    if $verb_at == -1 { print -e 'lane-commons: set-title argv lost the set-title verb'; exit 1 }
    if $server_at <= $verb_at {
        print -e $"lane-commons: --server at ($server_at) must come AFTER the set-title verb at ($verb_at) \(the run facade seats it on the verb segment\)"
        exit 1
    }
    # No --run: a pass script titles its OWN run via the engine-bound
    # FABRO_RUN_ID (the builder's contract, #1159 follow-through).
    assert-eq ($argv | any {|a| $a == '--run'}) false 'set-title argv carries no --run (FABRO_RUN_ID owns the target)'
    assert-eq ($argv | last) 'Watchdog: all enabled gears healthy' 'the title rides as the final positional'
}

def test-door-read-argv []: nothing -> nothing {
    # work#1150: the lane check's work-list and the watchdog's
    # automations-list/ps reads retired their inline literals into the
    # builders. Each pin freezes the RETIRED literal byte-for-byte —
    # these are the shapes the lane-check and stall-check smokes drive
    # against the channel shim.
    let srv = 'http://127.0.0.1:32276'
    assert-eq (work_channel work-list-argv $srv 'fabro-proof' 'develop') [
        'work' '--server' $srv 'list' '--project' 'fabro-proof' '--lane' 'develop' '--limit' '1' '--json'
    ] 'work-list-argv equals the retired lane-check literal (facade-level --server, subcommand before --project)'
    assert-eq (work_channel automations-list-argv $srv) [
        'automations' '--server' $srv 'list' '--json'
    ] 'automations-list-argv equals the retired stall-check literal (facade-level --server)'
    assert-eq (work_channel ps-argv $srv --all --workflow 'watchdog') [
        'ps' '--server' $srv '--all' '--json' '--workflow' 'watchdog'
    ] 'ps-argv equals the retired stall-check literal (verb-level --server)'
    # The bare ps read: no --all, no workflow filter — the caller-scoped
    # default the shim smoke's owned-ps arm drives.
    assert-eq (work_channel ps-argv $srv) [
        'ps' '--server' $srv '--json'
    ] 'ps-argv defaults to the caller-scoped json read'
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

    # The merger worklist literal (work#1126): the closable anti-join
    # restricted to merge:enabled — the argv the merger fast path reads
    # and probe-40's merger arm drives.
    let merger_queue = [
        'run' 'queue' '--field' 'closable' '--requires' 'merge=enabled'
        '--workflow' 'develop' '--json' '--server' 'http://127.0.0.1:32276'
    ]
    assert-eq (work_channel run-queue-argv 'http://127.0.0.1:32276' 'closable' 'develop' '' 'merge=enabled') $merger_queue 'run-queue-argv equals the merger worklist literal (work#1126)'

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
        let text = (consumer-src 'develop' $name)
        assert-contains $text 'use ../../../scripts/lib/work_channel.nu' $"($name) sources the work-channel module"
        assert-contains $text 'use ../../../scripts/lib/run_anchor.nu' $"($name) sources the run-anchor module"
    }
    for name in $CONSUMER_ANCHOR {
        let text = (consumer-src 'develop' $name)
        assert-contains $text 'use ../../../scripts/lib/run_anchor.nu' $"($name) sources the run-anchor module"
    }
    # work#1150: the two door consumers source the work-channel module —
    # their argv travels through the builders now.
    for row in $CONSUMER_LANES {
        let text = (consumer-src $row.workflow $row.name)
        assert-contains $text 'use ../../../scripts/lib/work_channel.nu' $"($row.name) sources the work-channel module"
    }
}

def test-no-mirror-regrows []: nothing -> nothing {
    # The mirror markers: any of these defs back in a consumer means a
    # copy regrew beside the lib (the state #1108 retired).
    let mirrors = ['def work-call' 'def work-server' 'def run-base' 'def current-branch' 'def current-run-id' 'def work-create']
    let names = ($CONSUMERS_BOTH | append $CONSUMER_ANCHOR)
    for name in $names {
        let text = (consumer-src 'develop' $name)
        for marker in $mirrors {
            if ($text | str contains $marker) {
                print -e $"lane-commons: mirror regrew in ($name): `($marker)` — the lib owns that shape now"
                exit 1
            }
        }
    }
}

# --- 4. extraction pins: no inline door argv (work#1150) -------------------

def assert-no-inline-server [workflow: string, name: string]: nothing -> nothing {
    let hits = (consumer-src $workflow $name | lines | enumerate | where {|it|
        (($it.item | str trim | str starts-with '#') == false) and ($it.item | str contains '--server')
    })
    if ($hits | is-not-empty) {
        for h in $hits {
            print -e $"lane-commons: inline --server argv regrew in ($workflow)/($name):($h.index + 1) — the work_channel builders own the flag, work#1150"
            print -e $"  ($h.item | str trim)"
        }
        exit 1
    }
}

def test-no-inline-server-argv []: nothing -> nothing {
    # The granularity hole (work#1150): test-no-mirror-regrows only
    # matches local `def`s, so an INLINE argv literal passed green —
    # exactly how the lane-check work-list and the stall-check
    # automations/ps literals landed. Every `--server` token on a
    # consumer's code line is a re-grown literal now: consumers resolve
    # the server with `server` and hand its VALUE to a builder. Comment
    # lines are prose, not argv.
    for name in ($CONSUMERS_BOTH | append $CONSUMER_ANCHOR) {
        assert-no-inline-server 'develop' $name
    }
    for row in $CONSUMER_LANES {
        assert-no-inline-server $row.workflow $row.name
    }
}

# --- 5. quote-guard pins: the recorded-check trap (work#1190) --------------

def test-recorded-check-quote-guard []: nothing -> nothing {
    # The trap: a nested `sh -c` payload parses twice (the wrapper's
    # bash, then the inner shell) and the inner layer's quoting intent is
    # invisible in the raw text — the recorded diagnostic of run
    # 01M6X3WCQ1 (work#1190) made the inner shell parse the unquoted
    # graphviz arrow as a redirect, and echo's output became the
    # repo-root file `b` in the evidence commit.
    #
    # REFUSED — the verbatim trap shape.
    assert-true (check-transcript nested-shell-metachar-trap 'sh -c "echo digraph G { a -> b } | ... __render-graph"') 'the #1190 trap refuses (nested shell, unquoted arrow in the payload)'
    # REFUSED — inner single-quoting changes nothing: the wrapper cannot
    # verify the inner layer's quoting from the raw text.
    assert-true (check-transcript nested-shell-metachar-trap "sh -c 'echo digraph G { a -> b }'") 'nested shell with an inner-quoted payload still refuses (conservative)'
    # ALLOWED — the demanded canonical form (demand shape 1): the DOT
    # single-quoted in a bare pipeline, one parse layer.
    assert-false (check-transcript nested-shell-metachar-trap "echo 'digraph G { a -> b }' | __render-graph") 'the single-quoted bare pipeline is the allowed canonical form'
    # ALLOWED — the heredoc escape.
    assert-false (check-transcript nested-shell-metachar-trap "__render-graph <<'EOF'\ndigraph G { a -> b }\nEOF") 'the heredoc escape is allowed'
    # ALLOWED — a deliberate top-level redirect parses exactly once, the
    # way it reads.
    assert-false (check-transcript nested-shell-metachar-trap 'cargo nextest run --workspace > /tmp/out.txt 2>&1') 'deliberate top-level redirects stay allowed'
    # ALLOWED — a nested shell WITHOUT metacharacters (the shape the
    # evidence smoke's re-run arms record).
    assert-false (check-transcript nested-shell-metachar-trap "sh -c 'exit 0'") 'nested shell without metacharacters is allowed'
    # ALLOWED — nu -c payloads are out of scope (nu pipes are the
    # idiomatic recording; the wrapper routes probes to nu script files).
    assert-false (check-transcript nested-shell-metachar-trap 'nu -c "open x | get y"') 'nu -c payloads are out of scope'
}

# --- 6. terminal-status predicate pins (work#1446, arch C2) -----------------
#
# The terminal family is GLOSSARY vocabulary: succeeded / failed /
# human_needed (work#1072, terminal) / dead, plus the canceled spellings
# the engine ACCEPTS but never EMITS (a cancelled run concludes `failed`
# with reason `cancelled` — petri FailureReason::Cancelled under
# fabro-types RunStatus::Failed; the wire kind set is exactly the four).
# Six consumers each spelled their own list until the predicate retired
# them (the decided semantics, Jev pre-screen 2026-11-18:
# terminal_not_blocking — human_needed counts as terminal EVERYWHERE
# including the push gate; held claims still block through the claim
# arm). Pin families:
#
#   7a. SPELLING pins — every canonical kind is terminal, both canceled
#       spellings accepted, and a renamed/unknown kind is NOT terminal
#       (the teeth: a platform-side rename lands as unknown → the gate
#       classifies the run active → fail closed, and this battery reds).
#   7b. GATE CLASSIFICATION pins — pure over captured ps/claims JSON:
#       parked-with-held-claim BLOCKS via the claim arm, parked-without-
#       claim does not; live runs block both arms; failed-run orphan
#       claims stay the reconciler's (the 2026-11-13 incident); empty
#       runId and unknown kinds fail closed.
#   7c. EXTRACTION pins — no quoted kind literal regrows in any of the
#       six consumers; every predicate consumer sources the module.

# The six consumers work#1446 retired (the pre-fix inline lists: push-gate
# succeeded/failed/canceled/cancelled — no human_needed, no dead;
# stall-check and the shim succeeded/failed/human_needed/dead; salvage-analysis
# succeeded/failed/canceled/cancelled; salvage-pull failed/cancelled/succeeded;
# run_workflow != succeeded).
const TERMINAL_CONSUMERS = [
    '.fabro/scripts/push-gate.nu'
    '.fabro/workflows/watchdog/scripts/stall-check.nu'
    '.fabro/scripts/salvage-analysis.nu'
    '.fabro/scripts/salvage-pull.nu'
    '.fabro/scripts/fabro-work-shim.nu'
    'scripts/run_workflow.nu'
]

def repo-src [rel: string]: nothing -> string {
    open --raw ($DIR | path join '..' '..' $rel)
}

def test-terminal-kind-spellings []: nothing -> nothing {
    # The engine-emitted four (fabro-types RunStatusKind, snake_case).
    for kind in [succeeded failed human_needed dead] {
        assert-true (is-terminal-kind $kind) $"terminal kind accepted: ($kind)"
    }
    # BOTH canceled spellings accepted — the platform's own FromStr takes
    # both ("canceled" | "cancelled"), and the pre-fix consumers spelled
    # them; the predicate never narrows an accepted spelling.
    assert-true (is-terminal-kind 'canceled') 'the one-L canceled spelling is accepted'
    assert-true (is-terminal-kind 'cancelled') 'the two-L cancelled spelling is accepted'
    assert-eq ($TERMINAL_KINDS | length) 6 'the terminal family carries six kinds'
    # Non-terminal kinds (RunStatusKind's active family) are NOT terminal.
    for kind in [submitted pending runnable starting running blocked paused removing] {
        assert-false (is-terminal-kind $kind) $"active kind rejected: ($kind)"
    }
    # THE TEETH (rename pin): a platform-side rename of a kind lands here
    # as an unknown string — never terminal, so the gate fails CLOSED and
    # this battery reds instead of silently passing a renamed vocabulary.
    for kind in [unknown 'succeded' 'canceld' ''] {
        assert-false (is-terminal-kind $kind) $"unknown kind rejected: ($kind)"
    }
    # The integration verdict (run_workflow's consumer): only a succeeded
    # run integrates.
    assert-true (is-success-kind 'succeeded') 'success kind accepted'
    assert-false (is-success-kind 'failed') 'failed is not success'
    assert-false (is-success-kind 'human_needed') 'a parked run is not a green run'
}

# A captured-ps-row fixture (the real `ps -a --json` row shape).
def ps-row [run_id: string, kind: string]: nothing -> record {
    {run_id: $run_id, workflow_slug: 'develop', status: {kind: $kind}, start_time: ''}
}

# A captured-claims-row fixture (the claims view: camelCase runId).
def claim-row [run_id: string, state: string]: nothing -> record {
    {claimId: $"cl-($run_id)", itemKey: 'denkhaus/work#1', runId: $run_id, lane: 'develop', state: $state}
}

def test-gate-classification-parked []: nothing -> nothing {
    # THE DECIDED SEMANTICS (work#1446, terminal_not_blocking): a run
    # parked human_needed never blocks pushes BY ITSELF; a held claim on
    # it still blocks through the claim arm (the reconciler releases only
    # a refused verdict, work#1072/#1419).
    let parked = '01PARKED0000000000000000AA'
    let held = (gate-classification [(ps-row $parked 'human_needed')] [(claim-row $parked 'granted')])
    assert-eq ($held.active | length) 0 'a parked run is terminal for the run arm (work#1446)'
    assert-eq ($held.blocking | length) 1 'parked-with-held-claim still blocks via the claim arm'
    # parked-without-claim: nothing holds — the gate opens.
    let free = (gate-classification [(ps-row $parked 'human_needed')] [])
    assert-eq ($free.active | length) 0 'parked-without-claim: no active run'
    assert-eq ($free.blocking | length) 0 'parked-without-claim does not block'
    # Any live claim state holds on a parked run (granted/active are the
    # work states, awaiting_merge the merger handoff).
    let awaiting = (gate-classification [(ps-row $parked 'human_needed')] [(claim-row $parked 'awaiting_merge')])
    assert-eq ($awaiting.blocking | length) 1 'an awaiting_merge claim on a parked run still blocks'
    # A released claim does not hold.
    let released = (gate-classification [(ps-row $parked 'human_needed')] [(claim-row $parked 'released')])
    assert-eq ($released.blocking | length) 0 'a released claim never blocks'
}

def test-gate-classification-live []: nothing -> nothing {
    # A live run blocks through the run arm; its claim blocks through the
    # claim arm (work genuinely in flight).
    let live = '01LIVERUN000000000000000AA'
    let running = (gate-classification [(ps-row $live 'running')] [(claim-row $live 'active')])
    assert-eq ($running.active | length) 1 'a running run is active for the run arm'
    assert-eq ($running.blocking | length) 1 'a claim on a running run blocks'
    # The blocked/paused/quota family stays active (fail closed posture).
    let blocked = (gate-classification [(ps-row $live 'blocked')] [])
    assert-eq ($blocked.active | length) 1 'a blocked run is active'
    let unknown = (gate-classification [(ps-row $live 'some_future_kind')] [])
    assert-eq ($unknown.active | length) 1 'an unknown kind fails the run arm CLOSED'
}

def test-gate-classification-healed []: nothing -> nothing {
    # Terminal-run orphan claims are the RECONCILER's to heal, never the
    # gate's to wait on (work_reconciler: run status is the truth; the
    # 2026-11-13 incident: a failed run's orphan claim held the trunk).
    for kind in [succeeded failed dead] {
        let done = '01DONERUN0000000000000000AA'
        let healed = (gate-classification [(ps-row $done $kind)] [(claim-row $done 'granted')])
        assert-eq ($healed.active | length) 0 $"a ($kind) run is terminal for the run arm"
        assert-eq ($healed.blocking | length) 0 $"a claim on a ($kind) run is the reconciler's to heal"
    }
    # An unresolvable run id (absent from the full ps listing — deleted or
    # archived run) stays reconciler territory, NOT a block.
    let ghost = (gate-classification [(ps-row '01DONERUN0000000000000000AA' 'succeeded')] [(claim-row '01GHOSTRUN000000000000000AA' 'granted')])
    assert-eq ($ghost.blocking | length) 0 'a claim on an unresolvable run id does not block'
    # FAIL-CLOSED posture untouched: a claim with NO run id blocks.
    let blind = (gate-classification [] [{claimId: 'cl-1', itemKey: 'denkhaus/work#1', runId: '', lane: 'develop', state: 'granted'}])
    assert-eq ($blind.blocking | length) 1 'a claim without a run id fails CLOSED'
    # The prefix truncation holds: `str substring 0..26` is INCLUSIVE on
    # this nu (a 27-char prefix), the retired consumer shape verbatim —
    # ids sharing the 27-char prefix resolve as the same run even where
    # they differ past it. Pinned so the truncation cannot silently
    # become exact-matching (which would strand real claims).
    let long = (gate-classification
        [(ps-row '01LIVE00000000000000000000AA' 'running')]
        [(claim-row '01LIVE00000000000000000000AB' 'granted')])
    assert-eq ($long.blocking | length) 1 'ids sharing the 27-char substring prefix resolve as the same run'
}

def test-consumers-use-the-predicate []: nothing -> nothing {
    # The kind-spelling scan: NO quoted kind literal may regrow inside a
    # list literal in any of the six consumers (non-comment lines) — the
    # terminal FAMILY (two-plus kinds in one construct) is lib vocabulary
    # alone (rg "'succeeded'" shows the list once). Single-kind compares
    # against a stage outcome (analyze-dump's green-lie probe,
    # `$o == 'failed'`) are not family vocabulary and stay local.
    for rel in $TERMINAL_CONSUMERS {
        let hits = (repo-src $rel | lines | enumerate | where {|it|
            (($it.item | str trim | str starts-with '#') == false) and ($it.item | parse --regex `\[[^\]\n]*['\"](succeeded|failed|canceled|cancelled|human_needed|dead)['\"]` | is-not-empty)
        })
        if ($hits | is-not-empty) {
            for h in $hits {
                print -e $"lane-commons: a quoted kind list regrew in ($rel):($h.index + 1) — lib/terminal_status.nu owns the vocabulary, work#1446"
                print -e $"  ($h.item | str trim)"
            }
            exit 1
        }
    }
    # The retired const/classification lines specifically (catches an
    # unquoted regrowth like salvage-analysis's old const).
    for rel in $TERMINAL_CONSUMERS {
        let hits = (repo-src $rel | lines | enumerate | where {|it|
            (($it.item | str trim | str starts-with '#') == false) and (
                ($it.item | str contains 'const TERMINAL') or ($it.item | str contains 'let terminal = [')
            )
        })
        if ($hits | is-not-empty) {
            for h in $hits {
                print -e $"lane-commons: an inline terminal list regrew in ($rel):($h.index + 1) — work#1446"
                print -e $"  ($h.item | str trim)"
            }
            exit 1
        }
    }
    # Every predicate consumer SOURCES the module through its OWN entry
    # point — the conversion shape pinned per consumer, so a re-grown
    # inline compare cannot hide behind an unused import.
    let entry_points = [
        [path entry];
        ['.fabro/scripts/push-gate.nu' 'gate-classification']
        ['.fabro/workflows/watchdog/scripts/stall-check.nu' 'is-terminal-kind']
        ['.fabro/scripts/salvage-pull.nu' 'is-terminal-kind']
        ['.fabro/scripts/fabro-work-shim.nu' 'is-terminal-kind']
        ['scripts/run_workflow.nu' 'is-success-kind']
        ['.fabro/scripts/salvage-sweep.nu' 'is-terminal-kind']
    ]
    for row in $entry_points {
        assert-contains (repo-src $row.path) 'terminal_status.nu' $"($row.path) sources the terminal-status module"
        assert-contains (repo-src $row.path) $row.entry $"($row.path) classifies through ($row.entry)"
    }
}

def test-human-needed-kind []: nothing -> nothing {
    # The adjudicator fast-path's single-kind predicate (the work#1446
    # note's same-file adoption): THE decision-request kind, spelled once
    # in the module — no consumer re-types the literal. Every other kind
    # (active, terminal, renamed, empty) is false by shape.
    assert-true (is-human-needed-kind 'human_needed') 'the decision-request kind is recognized'
    for kind in [succeeded failed dead canceled cancelled submitted running paused ''] {
        assert-false (is-human-needed-kind $kind) $"non-decision kind rejected: ($kind)"
    }
    assert-eq $HUMAN_NEEDED_KIND 'human_needed' 'the canonical spelling lives in the module'
}

def main []: nothing -> nothing {
    test-id-from-branch
    test-subject-mark
    test-work-create-argv
    test-observation-record-argv
    test-run-set-title-argv
    test-probe-literal-equality
    test-consumers-source-the-lib
    test-no-mirror-regrows
    test-no-inline-server-argv
    test-recorded-check-quote-guard
    test-terminal-kind-spellings
    test-gate-classification-parked
    test-gate-classification-live
    test-gate-classification-healed
    test-consumers-use-the-predicate
    test-human-needed-kind
    print 'lane-commons: all pins green'
}
