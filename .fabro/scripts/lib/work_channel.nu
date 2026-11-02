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
# stage), so closeout's wrapper exits non-zero on it.
export def observation-record [caller: string, category: string, text: string]: nothing -> any {
    let argv = (observation-record-argv (server $caller) $category $text)
    let exe = (bin)
    do { ^$exe ...$argv } | complete
}
