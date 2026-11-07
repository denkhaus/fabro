#!/usr/bin/env nu
# fabro-work-shim (denkhaus/work#990, reworked per-facade in #1109): the
# host-side channel shim for batteries — a state-file fake for the CLI
# facades so the lane scripts' channel contracts are pinnable without a
# server. The real channel exists only inside run sandboxes
# (FABRO_RUN_CHANNEL_SOCK is the run's mount); batteries point FABRO_BIN
# at THIS script and FABRO_WORK_SHIM_STATE at a JSON state file. Every
# invocation is appended to state.log (argv + the subcommand's OWN
# flags + stdin) so a smoke can assert the exact call sequence, and
# every mutation lands in the state file so a smoke can assert the
# durable outcome.
#
# PER-FACADE FLAG SCOPING (#1109): nu's script parser forces every
# accept-able flag into main's signature (a rest-only main rejects
# `--server` outright), so the union DECLARATION below is the
# interpreter's tax. The SEMANTICS stay per-facade: each subcommand
# owns exactly its own flag set, a flag from another facade's domain is
# refused loudly (`flag --x does not belong to <facade> <sub>`), and the
# log entry carries only that subcommand's flags — a new facade can no
# longer silently eat another facade's values.
#
# State shape (camelCase, mirroring the CLI's OpenAPI wire form):
#   {items: [{itemKey,title,body,state,labels}],
#    claims: [{claimId,itemKey,runId,lane,state}],
#    nextKey: <int>,
#    log: [{argv,flags,stdin}],
#    runs?: [{runId,workflowSlug,status,title?,pullRequest?,claimedItem?}],
#    marks?: [{runId,field,verdict,byRun}],
#    observations?: [{runId,seq,category,text}],
#    comments?: {"<itemKey>": [{author,body,createdAt?,url?}...]},
#    radius?: {"<itemKey>": [{itemKey,title,state}...]}}
#
# The adjudicator columns (#1107 stage D part 2): `run queue` renders
# RunQueueList over state.runs minus the runs state.marks settled
# (enabled/refused free a run; parked stays queued — the store's own
# semantics), `run mark` appends a verdict record, `observation list`
# serves a run's records, and `work radius` serves the fixture radius.
#
# It is a TEST ASSET: nothing in production may resolve the fabro binary
# here.

def state-path []: nothing -> string {
    let p = ($env.FABRO_WORK_SHIM_STATE? | default '')
    if ($p | is-empty) {
        print -e "fabro-work-shim: FABRO_WORK_SHIM_STATE is not set — batteries must provide the state file"
        exit 2
    }
    $p
}

def load-state [p: string]: nothing -> any {
    try { open --raw $p | from json } catch {
        print -e $"fabro-work-shim: state file unreadable: ($p)"
        exit 2
    }
}

def save-state [p: string, s: any]: nothing -> nothing {
    $s | to json --raw | save -f $p
}

# The LAST element of a list, null when empty (negative-index-free).
def last-or-null [list: list]: nothing -> any {
    let n = ($list | length)
    if $n == 0 { null } else { $list | get ($n - 1) }
}

# Refuse when any flag outside `owned` was provided: the per-facade
# scope guard (#1109). `provided` maps flag name to its bound value —
# an absent value-flag binds null, an absent switch binds false (nu's
# mechanics, verified): both count as absent here.
def refuse-foreign-flags [facade: string, sub: string, provided: record, owned: list<string>]: nothing -> nothing {
    for flag in ($provided | columns) {
        if ($flag in $owned) { continue }
        let value = ($provided | get $flag)
        if ($value != null and $value != false) {
            print -e $"fabro-work-shim: flag --($flag | str replace _ -) does not belong to ($facade) ($sub) — per-facade flag scoping"
            exit 2
        }
    }
}

# THE ARM SPECS (#1130, the sprint-78 architecture gate): one row per
# facade/subcommand pair, OWNED (the per-facade guard's list) and LOGGED
# (the log entry's flags) declared ADJACENTLY — the two per-arm sync
# points that lived pages apart (the signature, the provided record,
# the refuse list, the flags record) collapse to this row beside the
# arm body. The --all drift class (sprint-78's review fold: a flag that
# reached the signature and the arm but not the provided record) is
# structurally harder: a new flag touches the signature, the provided
# record, and THIS row — all three named in the enter path.
const ARMS = [
    [facade sub owned logged];
    ["work" "show" [server project json] [server project json]]
    ["work" "claims" [server project json] [server project json]]
    ["work" "list" [server project query limit lane json] [server project query json]]
    ["work" "create" [server project title type label priority body_file json] [server project title type label]]
    ["work" "close" [server project reason json] [server project reason]]
    ["work" "radius" [server project json] [server project json]]
    ["work" "comments" [server project json] [server project json]]
    ["run" "queue" [server field workflow verdict requires json] [server field workflow verdict requires json]]
    ["run" "merge" [server json] [server json]]
    ["run" "mark" [server json] [server json]]
    ["observation" "record" [server run_id category text execution firing json] [server run_id category text]]
    ["observation" "list" [server run_id json] [server run_id json]]
    ["automations" "list" [server json] [server json]]
    ["ps" "" [server workflow json all] [server workflow json all]]
]

# The spec row for one facade/subcommand pair; refuses loudly on an
# unknown pair (the match's fallback owns that path, this never misses).
def arm-spec [facade: string, sub: string]: nothing -> record {
    let row = ($ARMS | where {|a| $a.facade == $facade and $a.sub == $sub} | get -o 0 | default null)
    if $row == null {
        print -e $"fabro-work-shim: no arm spec for '($facade) ($sub)' — the ARMS table and the match must stay in lockstep"
        exit 2
    }
    $row
}

# The arm prologue (#1130): refuse foreign flags per the spec's OWNED
# list, build the log's flags record from the spec's LOGGED list, append
# the log entry, save, and return the logged state. Every arm body
# starts with `let logged = (enter ...)` and keeps only its semantics.
def enter [spec: record, p: string, s: any, argv: list<string>, stdin_text: string, provided: record]: nothing -> any {
    refuse-foreign-flags $spec.facade $spec.sub $provided $spec.owned
    let flags = ($provided | select ...$spec.logged)
    let logged = ($s | update log ($s.log | append {argv: $argv, flags: $flags, stdin: $stdin_text}))
    save-state $p $logged
    $logged
}

# main's signature is the interpreter-forced UNION of every facade's
# flags (see the header); dispatch scopes them per subcommand.
def main [
    --server: string,             # the facade-level transport flag
    # work
    --project: string,
    --query: string,
    --label: string,
    --limit: string,
    --reason: string,
    --title: string,
    --type: string,               # kebab-safe: binds $type (work create)
    --body-file: string,          # binds $body_file (work create)
    --priority: string,
    --lane: string,
    # run
    --field: string,              # run queue
    --workflow: string,           # run queue
    --verdict: string,            # run queue (the settled filter, stage E)
    --requires: string,           # run queue (the other-field requirement, work#1126)
    --all,                        # ps (the dead-man watch's pass read, stage F)
    # observation
    --run-id: string,             # binds $run_id (observation list/record)
    --category: string,           # observation record
    --text: string,               # observation record
    --execution: string,          # observation record
    --firing: string,             # observation record
    --json,
    ...argv: string               # the subcommand chain and positionals
]: nothing -> nothing {
    let p = (state-path)
    let stdin_text = (cat | str join)
    let s = (load-state $p)

    if ($argv | is-empty) {
        print -e $"fabro-work-shim: expected '<facade> [subcommand] [--server <srv>] ...', got nothing"
        exit 2
    }
    let facade = ($argv | get 0)
    # Single-facade verbs (ps, stage F) carry no subcommand: their flags
    # parse at the top level, so the second slot is ''.
    let sub = ($argv | get -o 1 | default '')
    let positional = ($argv | skip 2)
    let provided = {server: $server, project: $project, query: $query, label: $label, limit: $limit, reason: $reason, title: $title, type: $type, body_file: $body_file, priority: $priority, lane: $lane, field: $field, workflow: $workflow, verdict: $verdict, requires: $requires, run_id: $run_id, category: $category, text: $text, execution: $execution, firing: $firing, json: $json, all: $all}

    match [$facade $sub] {
        ["work" "show"] => {
            let logged = (enter (arm-spec "work" "show") $p $s $argv $stdin_text $provided)
            let key = ($positional | get -o 0 | default '')
            let work_item = ($logged.items | where {|it| $it.itemKey == $key} | get -o 0 | default null)
            if $work_item == null {
                print -e $"no work item '($key)' in the shim state"
                exit 1
            }
            if $json { $work_item | to json --raw | print } else { print $"($work_item.itemKey): ($work_item.title)" }
        }
        ["work" "claims"] => {
            let logged = (enter (arm-spec "work" "claims") $p $s $argv $stdin_text $provided)
            $logged.claims | to json --raw | print
        }
        ["work" "list"] => {
            let logged = (enter (arm-spec "work" "list") $p $s $argv $stdin_text $provided)
            let hits = ($logged.items | where {|it|
                ($it.state == "open") and (
                    ($query | is-empty) or ($it.title | str contains $query) or ($it.body | str contains $query)
                ) and (
                    # The lane filter (work#1144's lane check): the server
                    # translates --lane through the project's lane labels;
                    # the shim filters the same label.
                    ($lane | is-empty) or ($it.labels | any {|l| $l == $"lane:($lane)"})
                )
            })
            $hits | to json --raw | print
        }
        ["work" "create"] => {
            let logged = (enter (arm-spec "work" "create") $p $s $argv $stdin_text $provided)
            let n = ($logged | get -o nextKey | default 9000)
            let key = $"denkhaus/work#($n)"
            let labels = (if ($label | is-empty) { [] } else { $label | split row "," })
            let item_type = ($type | default 'task')
            let work_item = {itemKey: $key, title: ($title | default ''), body: $stdin_text, state: "open", labels: ($labels | append $"type:($item_type)")}
            save-state $p ($logged | update items ($logged.items | append $work_item) | update nextKey ($n + 1))
            print $key
        }
        ["work" "close"] => {
            let logged = (enter (arm-spec "work" "close") $p $s $argv $stdin_text $provided)
            let key = ($positional | get -o 0 | default '')
            let work_item = ($logged.items | where {|it| $it.itemKey == $key} | get -o 0 | default null)
            if $work_item == null {
                print -e $"no work item '($key)' in the shim state"
                exit 2
            }
            if $work_item.state == "closed" {
                # The door's idempotence refusal (a previous pass landed
                # it; the claim is gone) — the stable code rides the
                # rendered detail (fabro #1118).
                print -e $"item ($key) has no live claim — there is nothing to land [close_no_live_claim]"
                exit 1
            }
            let radius = ($logged | get -o radius | default {} | get -o $key | default [])
            if not ($radius | is-empty) {
                print -e $"item ($key) has an open closure radius [close_radius_open]"
                exit 1
            }
            let closed = ($logged.items | each {|it| if $it.itemKey == $key { $it | update state "closed" } else { $it } })
            save-state $p ($logged | update items $closed)
            print $"Closed ($key)."
        }
        ["work" "radius"] => {
            let logged = (enter (arm-spec "work" "radius") $p $s $argv $stdin_text $provided)
            let key = ($positional | get -o 0 | default '')
            let radius = ($logged | get -o radius | default {} | get -o $key | default [])
            let work_item = ($logged.items | where {|it| $it.itemKey == $key} | get -o 0 | default {itemKey: $key, title: "the fixture work item", body: "", state: "open", labels: []})
            {item: $work_item, radius: $radius} | to json --raw | print
        }
        ["work" "comments"] => {
            # The comment READ verb's fake (ADR-0039 stage D part 2): the
            # item must exist (the endpoint's 404 gate), then its
            # fixture comments come back oldest first, untouched.
            let logged = (enter (arm-spec "work" "comments") $p $s $argv $stdin_text $provided)
            let key = ($positional | get -o 0 | default '')
            let work_item = ($logged.items | where {|it| $it.itemKey == $key} | get -o 0 | default null)
            if $work_item == null {
                print -e $"no work item '($key)' in the shim state"
                exit 1
            }
            let comments = ($logged | get -o comments | default {} | get -o $key | default [])
            {comments: $comments} | to json --raw | print
        }
        ["run" "queue"] => {
            let logged = (enter (arm-spec "run" "queue") $p $s $argv $stdin_text $provided)
            let wanted_field = ($field | default "merge")
            let marks = ($logged | get -o marks | default [])
            let runs = ($logged | get -o runs | default [])
            let verdict_filter = ($verdict | default '')
            # The other-field requirement (work#1126): `<field>=<verdict>`
            # — only runs whose LATEST verdict on the OTHER field IS this
            # verdict list (the server-side worklist semantics).
            let requirement = ($requires | default '')
            let require_parts = (if ($requirement | is-empty) { null } else { $requirement | split row '=' })
            if ($require_parts != null) and (($require_parts | length) != 2) {
                print -e $"fabro-work-shim: run queue requires '<field>=<verdict>', got: ($requirement)"
                exit 2
            }
            let require_field = (if $require_parts == null { '' } else { $require_parts | get 0 })
            let require_verdict = (if $require_parts == null { '' } else { $require_parts | get 1 })
            let data = ($runs | where {|r|
                let latest = (last-or-null ($marks | where {|m| $m.runId == $r.runId and $m.field == $wanted_field}))
                let terminal = ($r.status in ["succeeded" "failed" "human_needed" "dead"])
                # Without a filter: the anti-join (pending or parked —
                # the settled runs left). With a filter (stage E):
                # exactly the runs settled on that verdict.
                let verdict_ok = if ($verdict_filter | is-empty) {
                    ($latest == null) or ($latest.verdict == "parked")
                } else {
                    ($latest != null) and ($latest.verdict == $verdict_filter)
                }
                # The requirement half: the OTHER field's latest verdict
                # must BE the required one (absent marks read pending).
                let require_ok = if $require_parts == null {
                    true
                } else {
                    let other = (last-or-null ($marks | where {|m| $m.runId == $r.runId and $m.field == $require_field}))
                    ($other != null) and ($other.verdict == $require_verdict)
                }
                $terminal and $verdict_ok and $require_ok and (($workflow | is-empty) or ($r.workflowSlug == $workflow))
            } | each {|r|
                let latest = (last-or-null ($marks | where {|m| $m.runId == $r.runId and $m.field == $wanted_field}))
                {run: {id: $r.runId, title: ($r | get -o title | default "the fixture run"), lifecycle: {status: {kind: $r.status}}, workflow: {slug: $r.workflowSlug}, pull_request: ($r | get -o pullRequest | default null)}, verdict: (if $latest == null { "pending" } else { $latest.verdict }), work_item: ($r | get -o claimedItem | default null), project: ($r | get -o project | default null)}
            })
            {data: $data, meta: {has_more: false, total: ($data | length)}} | to json --raw | print
        }
        ["run" "merge"] => {
            let logged = (enter (arm-spec "run" "merge") $p $s $argv $stdin_text $provided)
            let run_id = ($positional | get -o 0 | default '')
            let run = ($logged | get -o runs | default [] | where {|r| $r.runId == $run_id} | get -o 0 | default null)
            if $run == null {
                print -e $"fabro-work-shim: no fixture run '($run_id)'"
                exit 2
            }
            if ($run | get -o conflicted | default false) {
                # The door's conflict signal (the fast path's park
                # trigger): the stable code rides the rendered detail's
                # end, bracketed — the same format the real CLI prints
                # (fabro #1118).
                print -e $"the pull request cannot merge: conflict [merge_conflict]"
                exit 1
            }
            let merged = ($logged | update merges (($logged | get -o merges | default []) | append {runId: $run_id, at: (date now)}))
            save-state $p $merged
            let pr = ($run | get -o pullRequest | default null)
            let url = (if $pr == null { '' } else { $"https://github.com/($pr.owner)/($pr.repo)/pull/($pr.number)" })
            print $"run ($run_id) merged — ($url)"
        }
        ["run" "mark"] => {
            let logged = (enter (arm-spec "run" "mark") $p $s $argv $stdin_text $provided)
            let run_id = ($positional | get -o 0 | default '')
            let pair = ($positional | get -o 1 | default '')
            let parts = ($pair | split row '=')
            if ($run_id | is-empty) or ($parts | length) != 2 {
                print -e $"fabro-work-shim: run mark needs '<run-id> <field>=<verdict>', got: ($argv | str join ' ')"
                exit 2
            }
            let wanted_field = ($parts | get 0)
            let verdict = ($parts | get 1)
            if not ($wanted_field in ["merge" "closable"]) or not ($verdict in ["enabled" "refused" "parked"]) {
                print -e $"fabro-work-shim: unknown field/verdict in '($pair)' — fields merge|closable, verdicts enabled|refused|parked"
                exit 2
            }
            let writer = ($env.FABRO_RUN_ID? | default "01SHIM0000000000000000000")
            let marks = ($logged | get -o marks | default [])
            save-state $p ($logged | upsert marks ($marks | append {runId: $run_id, field: $wanted_field, verdict: $verdict, byRun: $writer}))
            print $"run ($run_id) marked ($wanted_field): ($verdict) \(by run ($writer)\)"
        }
        ["observation" "record"] => {
            let logged = (enter (arm-spec "observation" "record") $p $s $argv $stdin_text $provided)
            let claimed_run = ($run_id | default ($env.FABRO_RUN_ID? | default "shim"))
            let seen = ($logged | get -o observations | default [] | where {|o| $o.runId == $claimed_run})
            let next = (($seen | length) + 1)
            let all = ($logged | get -o observations | default [])
            save-state $p ($logged | upsert observations ($all | append {runId: $claimed_run, seq: $next, category: $category, text: $text}))
            print $"($category) recorded at seq ($next)"
        }
        ["observation" "list"] => {
            let logged = (enter (arm-spec "observation" "list") $p $s $argv $stdin_text $provided)
            let wanted_run = ($run_id | default '')
            let hits = ($logged | get -o observations | default [] | where {|o| $o.runId == $wanted_run} | each {|o| {seq: $o.seq, category: $o.category, text: $o.text}})
            {observations: $hits} | to json --raw | print
        }
        ["automations" "list"] => {
            # The dead-man watch's gear read (ADR-0039 stage F): the
            # fixture's automation rows render verbatim; the schedule
            # trigger's enabled flag is the stall semantics' input (a
            # disabled gear stays silent — the pause is authoritative).
            let logged = (enter (arm-spec "automations" "list") $p $s $argv $stdin_text $provided)
            let rows = ($logged | get -o automations | default [] | each {|a|
                {workflow: $a.workflow, triggers: [{type: "schedule", id: "schedule", enabled: $a.scheduleEnabled}]}
            })
            $rows | to json --raw | print
        }
        ["ps" ""] => {
            # The dead-man watch's pass read (stage F): the fixture's runs
            # render in the real `ps -a --json` row shape; start_time
            # derives from startedMinutesAgo so the smoke can pin fresh
            # and stale passes without wall-clock gymnastics.
            let logged = (enter (arm-spec "ps" "") $p $s $argv $stdin_text $provided)
            let rows = ($logged | get -o runs | default []
                | where {|r| ($workflow | is-empty) or ($r.workflowSlug == $workflow)}
                | each {|r|
                    let started = ((date now) - (($r | get -o startedMinutesAgo | default 60) * 1min))
                    {run_id: $r.runId, workflow_name: $r.workflowSlug, workflow_slug: $r.workflowSlug, status: {kind: $r.status}, start_time: ($started | format date "%+"), labels: []}
                })
            $rows | to json --raw | print
        }
        _ => {
            print -e $"fabro-work-shim: unsupported facade/subcommand '($facade) ($sub)'"
            exit 2
        }
    }
}
