#!/usr/bin/env nu
# The work channel (denkhaus/work#1108): the FABRO_BIN/FABRO_SERVER
# resolution and the work-facade call shape existed as MIRRORED copies
# in closeout.nu and evidence.nu, with stage C's record-fact built
# bespoke beside them. Every consumer — closeout, evidence, the
# adjudicator fast-path script (stage D), the stage-G reporter — uses
# THIS module; none re-implements the shape.
#
# Resolution contract (the mirrors' contract, unchanged):
#   FABRO_BIN    the fabro binary (host-side batteries shim it; the
#                channel only exists inside a run sandbox)
#   FABRO_SERVER the server the transport targets (set by the run's
#                channel mount in-sandbox; refused loudly when absent so
#                a host invocation never silently routes to a wrong
#                default)
#
# The argv BUILDERS are pure and exported (server injected as a
# parameter): fixtures pin the shapes here, and the fabro-test probe
# drives them against a captured-argv shim (probe-28 holds the
# parse-against-real-clap side). Transport wrappers add the binary,
# resolve the server, and return complete-wrapped records — callers own
# failure policy (advisory vs hard).

# The server the channel targets, `caller` naming the calling stage in
# the refusal (the operator must see WHICH stage failed). Exits 1 when
# FABRO_SERVER is unset.
export def server [caller: string = 'lane-commons']: nothing -> string {
    let srv = ($env.FABRO_SERVER? | default '' | str trim)
    if ($srv | is-empty) {
        print -e $"($caller): FABRO_SERVER is not set — the work channel needs it " + "(run sandboxes carry it via the channel mount)"
        exit 1
    }
    $srv
}

# Pure: the argv for one work-create call. `--project` belongs to CREATE
# (flags after the subcommand): at the `work` level clap only knows
# --server, and a pre-subcommand --project makes clap eat the verb as
# --server's value — run 01M60NX9W7N65DEPXYN09CE9BR lost a
# deferred-action filing to exactly that. Labels ride ONE comma-joined
# --label (the CLI accepts it in exactly this form; keeps the argv
# flat). The body travels on STDIN (--body-file -): tracker text never
# rides a command line.
export def create-argv [srv: string, title: string, item_type: string, labels: list<string>]: nothing -> list<string> {
    let flat = [
        'work'
        '--server'
        $srv
        'create'
        '--project'
        'fabro'
        '--title'
        $title
        '--body-file'
        '-'
        '--type'
        $item_type
        '--label'
        ($labels | str join ',')
    ]
    $flat
}

# Pure: the argv for one observation-record call (the script-stage door,
# ADR-0038/0039): flags mirror the `fabro_observation_record` tool
# parameters 1:1, no identity flags (the env quartet carries them).
export def observation-record-argv [srv: string, category: string, text: string]: nothing -> list<string> {
    [
        'observation'
        '--server'
        $srv
        'record'
        '--category'
        $category
        '--text'
        $text
    ]
}

# Pure: the argv for one run set-title call (the pass-title door,
# ADR-0030, the #1159 follow-through for PASS workflows): `--server`
# rides the VERB segment (the run facade seats it there, like run
# queue), the title is the positional, and NO --run — a pass script
# titles its OWN run through the engine-bound FABRO_RUN_ID.
export def run-set-title-argv [srv: string, title: string]: nothing -> list<string> {
    ['run' 'set-title' '--server' $srv $title]
}

# --- The adjudicator doors (ADR-0039 stage D part 2) ---------------------
# The run facade's grammar is verb-first: `fabro run` carries an optional
# WORKFLOW positional that eats a leading subcommand once a flag precedes
# it (verified against the real parser: `run --server X mark ...` resolves
# `mark` as the workflow selector), so the verb comes directly after
# `run` and --server rides the SUBCOMMAND segment. The observation and
# work facades keep the facade-level server (no positional precedes
# their subcommands).

# Pure: the argv for one run-queue read (the adjudicator's worklist).
# The optional settled-verdict filter (ADR-0039 stage E) flips the join:
# `--verdict enabled` on closable is the closing pass's worklist. The
# optional requirement (work#1126) restricts the anti-join to runs whose
# latest verdict on the OTHER field IS the named one: `--requires
# merge=enabled` on closable is the merger's worklist — a merge-pending
# or merge-refused run never reaches the merge door.
export def run-queue-argv [srv: string, field: string, workflow: string, verdict: string = '', requirement: string = '']: nothing -> list<string> {
    mut argv = ['run' 'queue' '--field' $field]
    if not ($requirement | is-empty) {
        $argv = ($argv | append ['--requires' $requirement])
    }
    if not ($verdict | is-empty) {
        $argv = ($argv | append ['--verdict' $verdict])
    }
    $argv = ($argv | append ['--workflow' $workflow '--json' '--server' $srv])
    $argv
}

# Pure: the queue entries from a run-queue call's stdout, [] on any
# parse failure (the caller's transport check already failed hard on a
# non-zero exit). One parser for every queue consumer — the adjudicator
# fast path, the closing pass, and the merger fast path.
export def entries-from [stdout: string]: nothing -> list<any> {
    let parsed = (try { $stdout | from json } catch { null })
    if ($parsed | describe | str starts-with "record") {
        $parsed | get -o data | default []
    } else { [] }
}

# Pure: the argv for one run-mark write (the verdict door). The argv
# stays the bare door grammar — `run mark <run-id> <field>=<verdict>`
# (the real parser and the OpenAPI mark request {field, verdict} carry
# no note flag) — so a park's WHY travels beside it: mark-run-hard's
# optional note rides the verdict-rationale observation channel, never
# this argv.
export def run-mark-argv [srv: string, run_id: string, field: string, verdict: string]: nothing -> list<string> {
    ['run' 'mark' $run_id $"($field)=($verdict)" '--server' $srv]
}

# Pure: the argv for one run-merge call (the merger's write door,
# ADR-0039 stage E): merge the target run's pull request server-side.
export def run-merge-argv [srv: string, run_id: string]: nothing -> list<string> {
    ['run' 'merge' $run_id '--server' $srv]
}

# Pure: the argv for one observation-list read (a run's facts).
export def observation-list-argv [srv: string, run_id: string]: nothing -> list<string> {
    ['observation' '--server' $srv 'list' '--run-id' $run_id '--json']
}

# Pure: the argv for one global observation-feed read (ADR-0044: the
# triager's worklist — every run's records with the derived triage
# state). Filters compose server-side; `--json` prints the
# GlobalObservationList record (`.observations` + `.total`), never a
# bare array.
export def observation-feed-argv [
    srv: string,
    --category: string,
    --triage: string,
    --run-id: string,
    --sort-by: string,
    --limit: int = 25,
    --page: int = 1,
]: nothing -> list<string> {
    mut argv = ['observation' '--server' $srv 'feed' '--json' '--limit' ($limit | into string) '--page' ($page | into string)]
    if not ($category | is-empty) { $argv = ($argv | append ['--category' $category]) }
    if not ($triage | is-empty) { $argv = ($argv | append ['--triage' $triage]) }
    if not ($run_id | is-empty) { $argv = ($argv | append ['--run-id' $run_id]) }
    if not ($sort_by | is-empty) { $argv = ($argv | append ['--sort' $sort_by]) }
    $argv
}

# Pure: the argv for one work-radius read (the closure radius of a
# work item; the CLI URL-encodes the item key's `#`).
export def work-radius-argv [srv: string, project: string, work_item: string]: nothing -> list<string> {
    ['work' '--server' $srv 'radius' '--project' $project $work_item '--json']
}

# Pure: the argv for one work-comments read (the ANSWER CHANNEL — a
# parked run's fork travels back as a comment on the work item,
# ADR-0039 stage D part 2).
export def work-comments-argv [srv: string, project: string, work_item: string]: nothing -> list<string> {
    ['work' '--server' $srv 'comments' '--project' $project $work_item '--json']
}

# Pure: the argv for one work-close write (the closing pass's door,
# ADR-0039 stage E): the adjudicator closes the merged run's item; the
# claim lands server-side (merge-complete).
export def work-close-argv [srv: string, project: string, work_item: string, reason: string]: nothing -> list<string> {
    ['work' '--server' $srv 'close' '--project' $project $work_item '--reason' $reason]
}

# --- The read doors of the pass scripts (denkhaus/work#1150) -------------
# The lane check and the watchdog's stall check mirrored these three argv
# inline — the registry's contract (every door's call shape lives HERE,
# #1108) had three holes. Pure builders beside their siblings; the
# lane-commons pin freezes each retired inline literal byte-for-byte and
# flags a re-grown `--server` in the consumer set.

# Pure: the argv for one work-list read (the lane check's drained-lane
# door, work#1144 + work#1150): the work facade seats --server at the
# FACADE level (like radius/comments/close), the subcommand precedes
# --project (run 01M60NX9's lesson), --lane rides the server-side label
# translation, and --limit bounds the read (the lane check wants the
# ANSWER — any open item — not the whole lane). --json prints the
# WorkItemListResponse envelope (.data + .total + .nextCursor), never a
# bare array (work#1476).
export def work-list-argv [srv: string, project: string, lane: string, limit: int = 1]: nothing -> list<string> {
    ['work' '--server' $srv 'list' '--project' $project '--lane' $lane '--limit' ($limit | into string) '--json']
}

# Pure: the argv for one cursor-paged work-list read (work#1476: the
# census door — friction-score enumerates the project's whole item
# population through the facade, the raw gh leg retired). The state
# spans every item (`all`); the cursor is the previous response's
# nextCursor verbatim, the first page omits it; --limit is the page
# size (the API clamps 1-100). --json prints the envelope, so the
# walker reads `.data` and `.nextCursor` per page.
export def work-list-page-argv [
    srv: string,
    project: string,
    --state: string = 'all',
    --cursor: string = '',
    --limit: int = 100,
]: nothing -> list<string> {
    mut argv = [
        'work' '--server' $srv 'list' '--project' $project
        '--state' $state '--limit' ($limit | into string) '--json'
    ]
    if not ($cursor | is-empty) { $argv = ($argv | append ['--cursor' $cursor]) }
    $argv
}

# Pure: the argv for one automations-list read (the watchdog's gear
# inventory, work#1150): the automations facade seats --server at the
# FACADE level (`fabro automations --server X list`); --json prints the
# automation rows.
export def automations-list-argv [srv: string]: nothing -> list<string> {
    ['automations' '--server' $srv 'list' '--json']
}

# Pure: the argv for one ps read (the watchdog's last-pass door,
# work#1150): ps is a TOP-LEVEL verb, --server rides the verb level;
# --all widens past the caller's own runs, --workflow filters
# server-side. --json is unconditional (every consumer parses records).
export def ps-argv [srv: string, --all, --workflow: string = '']: nothing -> list<string> {
    mut argv = ['ps' '--server' $srv]
    if $all { $argv = ($argv | append '--all') }
    $argv = ($argv | append '--json')
    if not ($workflow | is-empty) { $argv = ($argv | append ['--workflow' $workflow]) }
    $argv
}

# One CLI call over the channel with a COMPLETE argv (the builders
# above carry facade, server and flags in their grammatically correct
# positions — the run facade differs from work/observation). Returns the
# complete-wrapped result record; callers own failure policy.
export def cli [caller: string, argv: list<string>]: nothing -> any {
    let exe = (bin)
    do { ^$exe ...$argv } | complete
}

# Title the CALLING run with its sweep outcome (the #1159 follow-through
# for PASS workflows: the board tile names what the pass FOUND, not the
# generic goal). ADVISORY by design — a failed title write prints and
# never exits: the caller's own exit code is the contract, the title is
# the glanceable surface (it also carries door failures onto the board:
# #1165 — twenty green watchdog runs carried an auth failure only in
# their failure string). Bounded to the CLI's 100-character limit.
export def title-own-run [caller: string, title: string]: nothing -> nothing {
    let argv = (run-set-title-argv (server $caller) ($title | str substring 0..99))
    let res = (cli $caller $argv)
    if $res.exit_code != 0 {
        print -e $"($caller): run set-title failed \(advisory\): ($res.stderr | str trim | str substring 0..160)"
    }
}

# The fabro binary (host-side batteries shim it through FABRO_BIN).
def bin []: nothing -> string {
    $env.FABRO_BIN? | default 'fabro'
}

# One outbound work-facade call: the args list carries the subcommand
# and ITS flags verbatim (a rest-param would eat `--project` as THIS
# command's flag); this helper adds the binary and the server. Returns
# the complete-wrapped result record.
export def call [caller: string, args: list<string>]: nothing -> any {
    let exe = (bin)
    do { ^$exe work --server (server $caller) ...$args } | complete
}

# One work-create call with the body on STDIN. Returns the
# complete-wrapped result record; callers own the failure handling.
export def create [caller: string, title: string, body: string, item_type: string, labels: list<string>]: nothing -> any {
    let argv = (create-argv (server $caller) $title $item_type $labels)
    let exe = (bin)
    do { $body | ^$exe ...$argv } | complete
}

# One observation-record call (the script-stage door). Returns the
# complete-wrapped result record; callers own the failure policy — the
# reporter's facts are HARD by contract (a failed record fails the
# stage), so the reporter's wrapper exits non-zero on it.
export def observation-record [caller: string, category: string, text: string]: nothing -> any {
    let argv = (observation-record-argv (server $caller) $category $text)
    let exe = (bin)
    do { ^$exe ...$argv } | complete
}

# --- The hard pass-script policies (#1132, the sprint-78 architecture
# gate): the three pass scripts (adjudicator fast-path, closing pass,
# merger fast-path) chose IDENTICAL failure policies beside the pure
# builders — these wrappers are those policies, one home. The pure
# builders stay pure; an advisory caller keeps composing `cli` directly.

# The hard queue read (HARD: a failed read fails the stage — the engine
# retries the sweep; the anti-join makes re-sweeps gap-safe). Prints the
# worklist line and returns the entries.
export def read-queue-hard [caller: string, field: string, workflow: string, verdict: string = '', requirement: string = '']: nothing -> list<any> {
    let srv = (server $caller)
    let res = (cli $caller (run-queue-argv $srv $field $workflow $verdict $requirement))
    if $res.exit_code != 0 {
        print -e $"($caller): run queue read failed: ($res.stderr | str trim | str substring 0..240)"
        exit 1
    }
    let entries = (entries-from $res.stdout)
    print $"($caller): worklist holds ($entries | length) run\(s\) on ($field)/($workflow)"
    $entries
}

# The hard verdict write (HARD: a failed mark fails the stage — the
# engine retries the whole sweep; marks are idempotent at the fold, a
# re-sweep is gap-safe).
#
# The optional `note` (work#1419 code-review fold) carries the park's
# WHY verbatim. The mark door itself has NO note transport — the CLI
# grammar is `run mark <run-id> <field>=<verdict>` and the OpenAPI mark
# request is {field, verdict} — so the note rides the established
# verdict-rationale channel instead: one observation record on the
# CALLING run, `verdict: <target> <field>=<verdict> — <note>`, the
# machine head the agent pass and the verdict-note cleanup
# (denkhaus/work#1409) already disposition by. The record is HARD as
# the mark (a failed rationale fails the stage; the idempotent
# re-sweep re-writes it). An empty note keeps the bare verdict — the
# old behavior, byte-identical, so the merger fast path and the
# closing pass (callers that pass no note) are unaffected.
export def mark-run-hard [caller: string, field: string, run_id: string, verdict: string, note: string = '']: nothing -> nothing {
    let srv = (server $caller)
    let res = (cli $caller (run-mark-argv $srv $run_id $field $verdict))
    if $res.exit_code != 0 {
        print -e $"($caller): run mark failed for ($run_id) \(($verdict)\): ($res.stderr | str trim | str substring 0..240)"
        exit 1
    }
    print $"($caller): marked ($run_id) \($field = ($verdict)\)"
    let why = ($note | str trim)
    if ($why | is-empty) { return }
    # Plain-string concatenation: the note may carry parentheses (a
    # planner note names per-criterion checks) — never an interpolated
    # string over unreviewed text.
    let rationale = ('verdict: ' + $run_id + ' ' + $field + '=' + $verdict + ' — ' + $why)
    let rec = (cli $caller (observation-record-argv $srv 'note' $rationale))
    if $rec.exit_code != 0 {
        print -e $"($caller): verdict rationale record failed for ($run_id): ($rec.stderr | str trim | str substring 0..240)"
        exit 1
    }
}

# The routing envelope tail: the smoke drive prints the record bare (a
# smoke asserts it directly, no engine consumes it); the engine drive
# prints the routing envelope — the preferred label plus the record
# under `output.<key>` (the command-stage contract every pass script
# ends with).
export def emit-envelope [label: string, key: string, record: any, smoke: bool = false]: nothing -> nothing {
    if $smoke {
        $record | to json --raw | print
        return
    }
    {preferred_next_label: $label, context_updates: {($"output.($key)"): $record}} | to json --raw | print
}
