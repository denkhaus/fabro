#!/usr/bin/env nu
# import-work-items.nu — the cutover's FINAL state transfer (runbook stages
# 2-3, fabro-4fb4): reads the frozen `.seeds/issues.jsonl` and files every
# item on the work tracker through the fabro CLI, then verifies the FULL
# normalized diff against the source is zero.
#
# The importer never writes `.seeds` (the file is frozen; drift aborts the
# run). All writes go through `fabro work create/close` against the named
# project; bodies travel through `--body-file -` (stdin), never a command
# line. Resume is checkpoint-based: one JSONL line per confirmed step, so a
# crash mid-window loses nothing.
#
# Usage:
#   nu .fabro/scripts/import-work-items.nu --mode plan --project fabro-dryrun \
#     --server https://mirtuell.net --freeze-sha <sha256> [--source .seeds/issues.jsonl]
#   nu .fabro/scripts/import-work-items.nu --mode import ... [--max N] [--pacing-ms 750]
#   nu .fabro/scripts/import-work-items.nu --mode verify ... [--repo denkhaus/work-dryrun] \
#     [--whitelist 1,2] [--sample 30]

def main [
    --mode: string = 'plan'            # plan | import | verify
    --project: string                  # work project id (required)
    --server: string                   # fabro server URL (required)
    --freeze-sha: string = ''          # sha256 of the source at freeze (required)
    --freeze-commit: string = 'f62eac76a'  # the commit the freeze records
    --source: string = '.seeds/issues.jsonl'
    --repo: string = ''                # tracker repo (extras sweep + report only)
    --whitelist: string = ''           # known pre-existing numbers in the repo, comma-joined
    --checkpoint-dir: string = 'tmp/cutover'
    --fabro: string = ''               # fabro binary (default: ~/.fabro/bin/fabro)
    --max: int = 0                     # stop after N items (smoke runs); 0 = all
    --pacing-ms: int = 750             # pause between write ops
    --sample: int = 30                 # label-search spot-check size in verify
    --only: string = ''                # canonical mode: comma-joined ids to emit
]: nothing -> nothing {
    if ($project | is-empty) { fail 'import: --project is required' }
    if ($server | is-empty) { fail 'import: --server is required' }
    if ($freeze_sha | is-empty) { fail 'import: --freeze-sha is required (sha256 of the source file)' }
    let fabro_bin = (if ($fabro | is-empty) { '~/.fabro/bin/fabro' | path expand } else { $fabro })
    if not ($fabro_bin | path exists) { fail $"import: fabro binary not found at ($fabro_bin)" }
    if not ($source | path exists) { fail $"import: source not found: ($source)" }

    # freeze pin: the source must byte-match the recorded freeze sha at the
    # START; a moved file means the freeze record is wrong, not the file.
    let actual = (open --raw $source | hash sha256)
    if $actual != $freeze_sha {
        fail $"import: source sha256 ($actual | str substring 0..16)... != freeze sha ($freeze_sha | str substring 0..16)... — the file moved after the freeze record"
    }
    if not ($checkpoint_dir | path exists) { mkdir $checkpoint_dir }
    let checkpoint = ($checkpoint_dir | path join $"checkpoint-($project).jsonl")

    let items = (load-source $source)
    let plan = (build-plan $items $freeze_commit)
    let cp = (load-checkpoint $checkpoint)

    match $mode {
        'plan' => { show-plan $plan $cp }
        'import' => {
            run-import $plan $cp $checkpoint $project $server $fabro_bin $max $pacing_ms
            show-plan $plan (load-checkpoint $checkpoint)
        }
        'verify' => {
            run-verify $plan $checkpoint $project $server $fabro_bin $repo $whitelist $sample
        }
        'canonical' => {
            if ($only | is-empty) { fail 'import: --mode canonical needs --only <ids,comma-joined>' }
            write-canonical $plan $only $checkpoint_dir
        }
        _ => { fail $"import: unknown mode ($mode)" }
    }
}

def fail [msg: string]: nothing -> nothing {
    print -e $"╔══ IMPORT ALARM: ($msg) ══╗"
    exit 1
}

# --- source, plan, canonical form ------------------------------------------

def load-source [path: string]: nothing -> list<record> {
    open --raw $path
    | lines
    | where {|line| ($line | str trim) != '' }
    | each {|line| $line | from json }
}

# The import order (runbook): open first so the live queue exists before
# history, then closed; within each half by createdAt so the sequence is
# deterministic across reruns.
def build-plan [items: list<record>, freeze_commit: string]: nothing -> list<record> {
    let open = ($items | where {|i| $i.status != 'closed'} | sort-by createdAt)
    let closed = ($items | where {|i| $i.status == 'closed'} | sort-by createdAt)
    $open | append $closed
    | each {|i| canonical $i $freeze_commit}
}

# Lane mapping (runbook stage 3): @fabro/@agent -> lane:develop,
# @loop -> lane:loop, everything else (unassigned, human) -> NO lane label
# = human triage. Lanes apply to open items only.
def lane-of [item: record]: nothing -> string {
    if $item.status == 'closed' { return '' }
    match ($item.assignee? | default '' | str lowercase) {
        'fabro' => 'lane:develop'
        'agent' => 'lane:develop'
        'loop' => 'lane:loop'
        '@loop' => 'lane:loop'
        _ => ''
    }
}

# The canonical tracker form of one seed: labels, body, target state, close
# reason. This is the single source both the importer writes and the
# verifier expects — the gateway diff is canonical vs tracker, never
# ad-hoc.
def canonical [item: record, freeze_commit: string]: nothing -> record {
    let lane = (lane-of $item)
    let prio = ($item.priority | default 2)
    let type = ($item.type | default 'task')
    let base_labels = [
        $"seeds:($item.id)"
        $"prio:p($prio)"
        $"type:($type)"
    ]
    let labels_with_lane = (if ($lane | is-empty) { $base_labels } else { $base_labels | append $lane })
    let labels = ($labels_with_lane | append ($item.labels? | default []) | uniq | sort)

    let description = ($item.description? | default '' | str trim)
    let body_desc = (if ($description | is-empty) {
        '(No body in the source seed.)'
    } else {
        $description
    })

    let assignee = ($item.assignee? | default '' | str trim)
    let blocked_by = ($item.blockedBy? | default [] | str join ', ')
    let blocks = ($item.blocks? | default [] | str join ', ')

    mut trailers = [
        $"Legacy-ID: ($item.id)"
        $"Type: ($type)"
        $"Priority: P($prio)"
        $"Status-at-Freeze: ($item.status)"
    ]
    if not ($assignee | is-empty) { $trailers = ($trailers | append $"Assignee: ($assignee)") }
    if not ($lane | is-empty) { $trailers = ($trailers | append $"Lane: ($lane | str replace 'lane:' '')") }
    if ($item.createdAt? | is-not-empty) { $trailers = ($trailers | append $"Original-Created: ($item.createdAt)") }
    if ($item.updatedAt? | is-not-empty) { $trailers = ($trailers | append $"Original-Updated: ($item.updatedAt)") }
    if ($item.status == 'closed') and ($item.closedAt? | is-not-empty) {
        $trailers = ($trailers | append $"Original-Closed: ($item.closedAt)")
    }
    if not ($blocked_by | is-empty) { $trailers = ($trailers | append $"Blocked-By: ($blocked_by)") }
    if not ($blocks | is-empty) { $trailers = ($trailers | append $"Blocks: ($blocks)") }
    $trailers = ($trailers | append $"Migrated-From: denkhaus/fabro@($freeze_commit)")

    let close_reason = (close-reason-of $item)
    mut body_lines = [
        $body_desc
        ''
        '---'
        ''
        '<!-- seeds-import: v1 -->'
        ...$trailers
    ]
    if not ($close_reason | is-empty) {
        $body_lines = ($body_lines | append ['', '## Close reason', '', $close_reason])
    }
    {
        id: $item.id
        title: $item.title
        status: $item.status
        priority: $prio
        labels: $labels
        body: ($body_lines | str join "\n")
        close_reason: $close_reason
        expect_state: (if $item.status == 'closed' { 'closed' } else { 'open' })
        created_at: $item.createdAt
    }
}

# Top-level closeReason first, extensions.closeReason as the legacy
# fallback (9 items keep their reason only in extensions).
def close-reason-of [item: record]: nothing -> string {
    let top = ($item.closeReason? | default '')
    if not ($top | is-empty) { return $top }
    $item.extensions? | default {} | get -o closeReason | default ''
}

# --- checkpoint -------------------------------------------------------------

# Checkpoint JSONL: one line per confirmed step; the LAST line per id wins.
# Steps: {"step":"created"} then optionally {"step":"closed"}. A step
# already recorded is never repeated; a crash between create and close
# resumes as "close it".
def load-checkpoint [path: string]: nothing -> list<record> {
    if not ($path | path exists) { return [] }
    open --raw $path
    | lines
    | where {|line| ($line | str trim) != '' }
    | each {|line| $line | from json }
}

def checkpoint-state [cp: list<record>]: nothing -> record {
    mut state = {}
    for $line in $cp {
        if $line.step == 'created' {
            $state = ($state | upsert $line.id {key: $line.key, closed: false})
        } else if $line.step == 'closed' {
            $state = ($state | upsert $line.id {key: $line.key, closed: true})
        }
    }
    $state
}

def append-checkpoint [path: string, entry: record]: nothing -> nothing {
    ($entry | to json -r) ++ "\n" | save --append $path
}

# --- the fabro CLI wrapper ---------------------------------------------------

# Runs `fabro work <args>` with optional stdin; returns the completed
# record. Retries GitHub rate-limit signals with a long sleep — a 403 from
# GitHub did NOT execute the operation, so retrying the same op is safe.
def work-call [
    fabro: string
    server: string
    args: list<string>
    stdin: string = ''
    --retries: int = 5
]: nothing -> record {
    mut run = (run-once $fabro $server $args $stdin)
    mut attempt = 1
    loop {
        if $run.exit_code == 0 { break }
        let err = ($run.stderr | str trim)
        # PRIMARY rate exhaustion (5k/hr per installation) resets up to an
        # hour out — wait in 5-minute steps. Secondary/abuse signals clear
        # in ~70s.
        let primary = ($err | str contains 'rate limit exceeded')
        let rate = $primary or ($err | str contains 'rate limit') or ($err | str contains '403') or ($err | str contains 'secondary limit') or ($err | str contains 'abuse')
        if (not $rate) or ($attempt >= $retries) { break }
        let wait = (if $primary { 300sec } else { 70sec })
        print -e $"  rate-limited, attempt ($attempt); sleeping ($wait): ($err | str substring 0..120)"
        sleep $wait
        $attempt = ($attempt + 1)
        $run = (run-once $fabro $server $args $stdin)
    }
    $run
}

def run-once [fabro: string, server: string, args: list<string>, stdin: string]: nothing -> record {
    if ($stdin | is-empty) {
        do { ^$fabro work --server $server ...$args } | complete
    } else {
        do { $stdin | ^$fabro work --server $server ...$args } | complete
    }
}

# --- import ------------------------------------------------------------------

def run-import [
    plan: list<record>
    cp: list<record>
    checkpoint: string
    project: string
    server: string
    fabro: string
    max: int
    pacing_ms: int
]: nothing -> nothing {
    let state = (checkpoint-state $cp)
    let todo = ($plan | where {|p| ($state | get -o $p.id) == null})
    let backlog = ($plan | where {|p|
        let s = ($state | get -o $p.id)
        if $s == null { false } else { (not $s.closed) and ($p.expect_state == 'closed') }
    })
    print $"== import: ($plan | length) source items | ($todo | length) to create | ($backlog | length) half-done closes | project ($project)"
    mut done = 0
    mut failures = 0
    let limit = (if $max > 0 { $max } else { -1 })

    # finish half-done items first (created but not closed)
    for $p in $backlog {
        if $limit >= 0 and $done >= $limit { break }
        let key = ($state | get $p.id | get key)
        if (close-one $fabro $server $project $key $p) {
            append-checkpoint $checkpoint {id: $p.id, step: 'closed', key: $key}
        } else {
            $failures = ($failures + 1)
        }
        $done = ($done + 1)
        sleep ($pacing_ms * 1ms)
    }

    for $p in $todo {
        if $limit >= 0 and $done >= $limit { break }
        let create_args = (['create' '--project' $project '--title' $p.title '--body-file' '-' '--label' ($p.labels | str join ',') '--json'])
        let run = (work-call $fabro $server $create_args $p.body)
        if $run.exit_code != 0 {
            print -e $"  CREATE FAILED ($p.id): ($run.stderr | str trim | str substring 0..200)"
            $failures = ($failures + 1)
            $done = ($done + 1)
            sleep ($pacing_ms * 1ms)
            continue
        }
        let key = ($run.stdout | from json | get itemKey)
        append-checkpoint $checkpoint {id: $p.id, step: 'created', key: $key}
        if $p.expect_state == 'closed' {
            if (close-one $fabro $server $project $key $p) {
                append-checkpoint $checkpoint {id: $p.id, step: 'closed', key: $key}
            }
        }
        $done = ($done + 1)
        if ($done mod 25) == 0 { print $"  ... ($done) processed this run" }
        sleep ($pacing_ms * 1ms)
    }
    print $"== import pass done: ($done) processed this run, ($failures) failures — any failure fails verify"
}

def close-one [fabro: string, server: string, project: string, key: string, p: record]: nothing -> bool {
    # `reason` is required by the API schema (and the CLI mirrors it), so a
    # closed item with no recorded reason closes with the standard
    # placeholder — the canonical body carries the truth (no close-reason
    # section); the comment is the tracker-side close note.
    let reason = (if ($p.close_reason | is-empty) {
        'Closed at the .seeds freeze; no close reason recorded.'
    } else {
        $p.close_reason
    })
    let run = (work-call $fabro $server ['close' '--project' $project $key '--reason' $reason])
    if $run.exit_code != 0 {
        print -e $"  CLOSE FAILED ($p.id) ($key): ($run.stderr | str trim | str substring 0..200)"
        return false
    }
    true
}

# --- verify (the gateway) -----------------------------------------------------

# Reads EVERY item back by number and diffs against canonical. The FULL
# normalized diff must be zero: title, body (byte-exact — canonical is the
# expectation), label SET (modulo the project label the registry owns),
# state, priority (roundtrip through the prio:pN label). Plus a label-search
# spot check (the property the stage-1a seam resolves through) and a gh
# extras sweep (nothing in the tracker the checkpoint cannot account for).
def run-verify [
    plan: list<record>
    checkpoint: string
    project: string
    server: string
    fabro: string
    repo: string
    whitelist: string
    sample: int
]: nothing -> nothing {
    let cp = (load-checkpoint $checkpoint)
    let state = (checkpoint-state $cp)
    let with_keys = ($plan | each {|p|
        let s = ($state | get -o $p.id)
        {expected: $p, key: (if $s == null { '' } else { $s.key })}
    })
    let missing = ($with_keys | where {|x| $x.key == ''})
    if ($missing | length) > 0 {
        print -e $"== VERIFY ALARM: ($missing | length) items have no checkpoint key — import incomplete; first 10:"
        for $m in ($missing | first 10) { print -e $"   ($m.expected.id)" }
    }
    print $"== verify: ($plan | length) source items, project ($project)"

    let proj_label = $"project:($project)"
    let targets = ($with_keys | where {|x| $x.key != ''})
    mut diffs = []
    mut read_failures = []
    mut n = 0
    for $t in $targets {
        $n = ($n + 1)
        let run = (do { ^$fabro work --server $server show --project $project $t.key --json } | complete)
        if $run.exit_code != 0 {
            $read_failures = ($read_failures | append {id: $t.expected.id, key: $t.key, error: ($run.stderr | str trim | str substring 0..160)})
            continue
        }
        let item = ($run.stdout | from json)
        let got_labels = ($item.labels | where {|l| $l != $proj_label} | sort)
        let want_labels = ($t.expected.labels | sort)
        if $got_labels != $want_labels {
            $diffs = ($diffs | append {id: $t.expected.id, field: 'labels', want: ($want_labels | str join ','), got: ($got_labels | str join ',')})
        }
        if $item.title != $t.expected.title {
            $diffs = ($diffs | append {id: $t.expected.id, field: 'title', want: $t.expected.title, got: $item.title})
        }
        if $item.body != $t.expected.body {
            $diffs = ($diffs | append {id: $t.expected.id, field: 'body', want: '(canonical)', got: ($item.body | str substring 0..120)})
        }
        if $item.state != $t.expected.expect_state {
            $diffs = ($diffs | append {id: $t.expected.id, field: 'state', want: $t.expected.expect_state, got: $item.state})
        }
        if $item.priority != $t.expected.priority {
            $diffs = ($diffs | append {id: $t.expected.id, field: 'priority', want: ($t.expected.priority | into string), got: ($item.priority | into string)})
        }
        if ($n mod 50) == 0 { print $"  ... ($n) read back" }
    }

    # label-search spot check: the stage-1a seam resolves through label
    # search — evenly spread sampled ids must return exactly one hit, the
    # right item. (30/min search budget: sample, not census.)
    mut search_bad = []
    if $sample > 0 and ($targets | length) > 0 {
        let spread = (spread-sample ($targets | each {|x| {id: $x.expected.id, key: $x.key}}) $sample)
        for $pair in $spread {
            let query = ('label:' ++ '"' ++ $"seeds:($pair.id)" ++ '"')
            let run = (do { ^$fabro work --server $server list --project $project --query $query --json } | complete)
            if $run.exit_code != 0 {
                $search_bad = ($search_bad | append {id: $pair.id, error: 'search call failed'})
                continue
            }
            let hits = (try { $run.stdout | from json } catch { [] })
            if (($hits | where {|h| $h.itemKey == $pair.key}) | length) != 1 {
                $search_bad = ($search_bad | append {id: $pair.id, error: $"label search: ($hits | length) hits, ($hits | where {|h| $h.itemKey == $pair.key} | length) matching ($pair.key)"})
            }
        }
    }

    # extras sweep (gh, audit path): nothing carrying the project label
    # that the checkpoint cannot account for (minus the whitelisted
    # pre-existing numbers).
    mut extras = []
    if not ($repo | is-empty) {
        let known = ($with_keys | where {|x| $x.key != ''} | each {|x| $x.key | split row '#' | last | into int} | append (
            if ($whitelist | is-empty) { [] } else { $whitelist | split row ',' | each {|n| $n | into int} }
        ))
        mut page = 1
        loop {
            let page_now = $page
            let fetch = (do { gh api $"repos/($repo)/issues?state=all&per_page=100&page=($page_now)" --jq '.[] | select(.pull_request == null) | {number, labels: [.labels[].name]}' } | complete)
            if $fetch.exit_code != 0 { break }
            let rows = (try { $fetch.stdout | from json } catch { [] })
            if ($rows | is-empty) { break }
            for $row in $rows {
                if ($row.labels | any {|l| $l == $proj_label}) and not ($row.number in $known) {
                    $extras = ($extras | append {number: $row.number, why: 'carries the project label, not in checkpoint or whitelist'})
                }
            }
            if ($rows | length) < 100 { break }
            $page = ($page + 1)
        }
    }

    let report = ($checkpoint | path dirname | path join $"report-($project).md")
    let summary = [
        $"# import verify report — project ($project) — (date now | format date '%Y-%m-%d %H:%M:%S %Z')"
        ''
        $"source items: ($plan | length)"
        $"read back: ($targets | length)"
        $"missing keys: ($missing | length)"
        $"read failures: ($read_failures | length)"
        $"normalized diffs: ($diffs | length)"
        $"label-search spot checks: (if $sample > 0 { $sample } else { 0 }), failures: ($search_bad | length)"
        $"extras beyond checkpoint: ($extras | length)"
    ]
    let diff_lines = (if ($diffs | is-empty) { ['## Diffs', '', 'none — FULL normalized diff is 0'] } else {
        ['## Diffs', ''] | append ($diffs | each {|d| $"- ($d.id) [($d.field)] want: ($d.want) | got: ($d.got)"} | first 40)
    })
    let read_lines = (if ($read_failures | is-empty) { [] } else {
        ['', '## Read failures', ''] | append ($read_failures | each {|f| $"- ($f.id) ($f.key): ($f.error)"} | first 20)
    })
    let search_lines = (if ($search_bad | is-empty) { [] } else {
        ['', '## Label-search failures', ''] | append ($search_bad | each {|s| $"- ($s.id): ($s.error)"} | first 20)
    })
    let extra_lines = (if ($extras | is-empty) { [] } else {
        ['', '## Extras', ''] | append ($extras | each {|e| $"- #($e.number): ($e.why)"} | first 20)
    })
    let body = ($summary | append $diff_lines | append $read_lines | append $search_lines | append $extra_lines | str join "\n")
    $"($body)\n" | save --force $report
    print $body

    if (not ($diffs | is-empty)) or (not ($missing | is-empty)) or (not ($read_failures | is-empty)) or (not ($search_bad | is-empty)) or (not ($extras | is-empty)) {
        print -e $"== VERIFY FAILED: see ($report)"
        exit 1
    }
    print '== VERIFY PASSED: FULL normalized diff of 0'
}

# Deterministic even spread: sample_size indices across the list.
def spread-sample [rows: list<record>, size: int]: nothing -> list<record> {
    let len = ($rows | length)
    if $len == 0 { return [] }
    let take = (if $size > $len { $len } else { $size })
    if $take <= 0 { return [] }
    seq 0 ($take - 1) | each {|i| $rows | get (($i * $len) // $take)}
}

# --- canonical emission (pilot retrofit) --------------------------------------

# Emits the canonical body for the named ids, one file per id. The two
# hand-migrated pilots (fabro-d5f0 -> #6, fabro-9337 -> #8) predate the
# label amendment; the retrofit applies the same canonical form the
# importer writes so the gateway diff covers them too.
def write-canonical [plan: list<record>, only: string, out_dir: string]: nothing -> nothing {
    let ids = ($only | split row ',')
    for $id in $ids {
        let p = ($plan | where {|p| $p.id == $id} | first)
        if $p == null { fail $"canonical: ($id) not in the source" }
        let path = ($out_dir | path join $"canonical-($id).md")
        $"($p.body)\n" | save --force $path
        print $"canonical: ($id) -> ($path)"
        print $"  labels: ($p.labels | str join ',')  expect state: ($p.expect_state)"
    }
}

# --- plan display ---------------------------------------------------------------

def show-plan [plan: list<record>, cp: list<record>]: nothing -> nothing {
    let state = (checkpoint-state $cp)
    let created = ($plan | where {|p| ($state | get -o $p.id) != null})
    let closed = ($plan | where {|p|
        ($state | get -o $p.id | default {} | get -o closed | default false)
    })
    let lanes = ($plan | where {|p| $p.expect_state == 'open'} | each {|p|
        if ('lane:develop' in $p.labels) { 'develop' } else if ('lane:loop' in $p.labels) { 'loop' } else { 'no-lane' }
    })
    let open_count = ($plan | where {|p| $p.expect_state == 'open'} | length)
    let dev_count = ($lanes | where {|l| $l == 'develop'} | length)
    let loop_count = ($lanes | where {|l| $l == 'loop'} | length)
    let no_lane = ($lanes | where {|l| $l == 'no-lane'} | length)
    print $"== plan: ($plan | length) items"
    print $"   open: ($open_count) — develop ($dev_count), loop ($loop_count), no-lane ($no_lane)"
    let closed_count = ($plan | where {|p| $p.expect_state == 'closed'} | length)
    let with_reason = ($plan | where {|p| $p.expect_state == 'closed' and ($p.close_reason | is-not-empty)} | length)
    print $"   closed: ($closed_count) — with close reason ($with_reason)"
    print $"   checkpoint: created ($created | length), fully closed ($closed | length), remaining (($plan | length) - ($created | length))"
}
