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

    if ($argv | length) < 2 {
        print -e $"fabro-work-shim: expected '<facade> <subcommand> [--server <srv>] ...', got: ($argv | str join ' ')"
        exit 2
    }
    let facade = ($argv | get 0)
    let sub = ($argv | get 1)
    let positional = ($argv | skip 2)
    let provided = {server: $server, project: $project, query: $query, label: $label, limit: $limit, reason: $reason, title: $title, type: $type, body_file: $body_file, priority: $priority, lane: $lane, field: $field, workflow: $workflow, verdict: $verdict, run_id: $run_id, category: $category, text: $text, execution: $execution, firing: $firing, json: $json}

    match [$facade $sub] {
        ["work" "show"] => {
            refuse-foreign-flags $facade $sub $provided ["server" "project" "json"]
            let flags = {server: $server, project: $project, json: $json}
            let logged = ($s | update log ($s.log | append {argv: $argv, flags: $flags, stdin: $stdin_text}))
            save-state $p $logged
            let key = ($positional | get -o 0 | default '')
            let work_item = ($logged.items | where {|it| $it.itemKey == $key} | get -o 0 | default null)
            if $work_item == null {
                print -e $"no work item '($key)' in the shim state"
                exit 1
            }
            if $json { $work_item | to json --raw | print } else { print $"($work_item.itemKey): ($work_item.title)" }
        }
        ["work" "claims"] => {
            refuse-foreign-flags $facade $sub $provided ["server" "project" "json"]
            let flags = {server: $server, project: $project, json: $json}
            let logged = ($s | update log ($s.log | append {argv: $argv, flags: $flags, stdin: $stdin_text}))
            save-state $p $logged
            $logged.claims | to json --raw | print
        }
        ["work" "list"] => {
            refuse-foreign-flags $facade $sub $provided ["server" "project" "query" "limit" "lane" "json"]
            let flags = {server: $server, project: $project, query: $query, json: $json}
            let logged = ($s | update log ($s.log | append {argv: $argv, flags: $flags, stdin: $stdin_text}))
            save-state $p $logged
            let hits = ($logged.items | where {|it|
                ($it.state == "open") and (
                    ($query | is-empty) or ($it.title | str contains $query) or ($it.body | str contains $query)
                )
            })
            $hits | to json --raw | print
        }
        ["work" "create"] => {
            refuse-foreign-flags $facade $sub $provided ["server" "project" "title" "type" "label" "priority" "body_file" "json"]
            let flags = {server: $server, project: $project, title: $title, type: $type, label: $label}
            let logged = ($s | update log ($s.log | append {argv: $argv, flags: $flags, stdin: $stdin_text}))
            let n = ($logged | get -o nextKey | default 9000)
            let key = $"denkhaus/work#($n)"
            let labels = (if ($label | is-empty) { [] } else { $label | split row "," })
            let item_type = ($type | default 'task')
            let work_item = {itemKey: $key, title: ($title | default ''), body: $stdin_text, state: "open", labels: ($labels | append $"type:($item_type)")}
            save-state $p ($logged | update items ($logged.items | append $work_item) | update nextKey ($n + 1))
            print $key
        }
        ["work" "close"] => {
            refuse-foreign-flags $facade $sub $provided ["server" "project" "reason" "json"]
            let flags = {server: $server, project: $project, reason: $reason}
            let logged = ($s | update log ($s.log | append {argv: $argv, flags: $flags, stdin: $stdin_text}))
            save-state $p $logged
            let key = ($positional | get -o 0 | default '')
            let work_item = ($logged.items | where {|it| $it.itemKey == $key} | get -o 0 | default null)
            if $work_item == null {
                print -e $"no work item '($key)' in the shim state"
                exit 2
            }
            if $work_item.state == "closed" {
                # The door's idempotence refusal (a previous pass landed
                # it; the claim is gone).
                print -e $"item ($key) has no live claim — there is nothing to land"
                exit 1
            }
            let radius = ($logged | get -o radius | default {} | get -o $key | default [])
            if not ($radius | is-empty) {
                print -e $"item ($key) has an open closure radius"
                exit 1
            }
            let closed = ($logged.items | each {|it| if $it.itemKey == $key { $it | update state "closed" } else { $it } })
            save-state $p ($logged | update items $closed)
            print $"Closed ($key)."
        }
        ["work" "radius"] => {
            refuse-foreign-flags $facade $sub $provided ["server" "project" "json"]
            let flags = {server: $server, project: $project, json: $json}
            let logged = ($s | update log ($s.log | append {argv: $argv, flags: $flags, stdin: $stdin_text}))
            save-state $p $logged
            let key = ($positional | get -o 0 | default '')
            let radius = ($logged | get -o radius | default {} | get -o $key | default [])
            let work_item = ($logged.items | where {|it| $it.itemKey == $key} | get -o 0 | default {itemKey: $key, title: "the fixture work item", body: "", state: "open", labels: []})
            {item: $work_item, radius: $radius} | to json --raw | print
        }
        ["work" "comments"] => {
            # The comment READ verb's fake (ADR-0039 stage D part 2): the
            # item must exist (the endpoint's 404 gate), then its
            # fixture comments come back oldest first, untouched.
            refuse-foreign-flags $facade $sub $provided ["server" "project" "json"]
            let flags = {server: $server, project: $project, json: $json}
            let logged = ($s | update log ($s.log | append {argv: $argv, flags: $flags, stdin: $stdin_text}))
            save-state $p $logged
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
            refuse-foreign-flags $facade $sub $provided ["server" "field" "workflow" "verdict" "json"]
            let flags = {server: $server, field: $field, workflow: $workflow, verdict: $verdict, json: $json}
            let logged = ($s | update log ($s.log | append {argv: $argv, flags: $flags, stdin: $stdin_text}))
            save-state $p $logged
            let wanted_field = ($field | default "merge")
            let marks = ($logged | get -o marks | default [])
            let runs = ($logged | get -o runs | default [])
            let verdict_filter = ($verdict | default '')
            let data = ($runs | where {|r|
                let latest = (last-or-null ($marks | where {|m| $m.runId == $r.runId and $m.field == $wanted_field}))
                let terminal = ($r.status in ["succeeded" "failed" "dead"])
                # Without a filter: the anti-join (pending or parked —
                # the settled runs left). With a filter (stage E):
                # exactly the runs settled on that verdict.
                let verdict_ok = if ($verdict_filter | is-empty) {
                    ($latest == null) or ($latest.verdict == "parked")
                } else {
                    ($latest != null) and ($latest.verdict == $verdict_filter)
                }
                $terminal and $verdict_ok and (($workflow | is-empty) or ($r.workflowSlug == $workflow))
            } | each {|r|
                let latest = (last-or-null ($marks | where {|m| $m.runId == $r.runId and $m.field == $wanted_field}))
                {run: {id: $r.runId, title: ($r | get -o title | default "the fixture run"), lifecycle: {status: $r.status}, workflow: {slug: $r.workflowSlug}, pull_request: ($r | get -o pullRequest | default null)}, verdict: (if $latest == null { "pending" } else { $latest.verdict }), work_item: ($r | get -o claimedItem | default null), project: ($r | get -o project | default null)}
            })
            {data: $data, meta: {has_more: false, total: ($data | length)}} | to json --raw | print
        }
        ["run" "merge"] => {
            refuse-foreign-flags $facade $sub $provided ["server" "json"]
            let flags = {server: $server, json: $json}
            let logged = ($s | update log ($s.log | append {argv: $argv, flags: $flags, stdin: $stdin_text}))
            save-state $p $logged
            let run_id = ($positional | get -o 0 | default '')
            let run = ($logged | get -o runs | default [] | where {|r| $r.runId == $run_id} | get -o 0 | default null)
            if $run == null {
                print -e $"fabro-work-shim: no fixture run '($run_id)'"
                exit 2
            }
            if ($run | get -o conflicted | default false) {
                # The door's conflict signal (the fast path's park
                # trigger): the refusal names the code.
                print -e $"the pull request cannot merge: conflict — code merge_conflict"
                exit 1
            }
            let merged = ($logged | update merges (($logged | get -o merges | default []) | append {runId: $run_id, at: (date now)}))
            save-state $p $merged
            let pr = ($run | get -o pullRequest | default null)
            let url = (if $pr == null { '' } else { $"https://github.com/($pr.owner)/($pr.repo)/pull/($pr.number)" })
            print $"run ($run_id) merged — ($url)"
        }
        ["run" "mark"] => {
            refuse-foreign-flags $facade $sub $provided ["server" "json"]
            let flags = {server: $server, json: $json}
            let logged = ($s | update log ($s.log | append {argv: $argv, flags: $flags, stdin: $stdin_text}))
            save-state $p $logged
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
            refuse-foreign-flags $facade $sub $provided ["server" "run_id" "category" "text" "execution" "firing" "json"]
            let flags = {server: $server, run_id: $run_id, category: $category, text: $text}
            let logged = ($s | update log ($s.log | append {argv: $argv, flags: $flags, stdin: $stdin_text}))
            let claimed_run = ($run_id | default ($env.FABRO_RUN_ID? | default "shim"))
            let seen = ($logged | get -o observations | default [] | where {|o| $o.runId == $claimed_run})
            let next = (($seen | length) + 1)
            let all = ($logged | get -o observations | default [])
            save-state $p ($logged | upsert observations ($all | append {runId: $claimed_run, seq: $next, category: $category, text: $text}))
            print $"($category) recorded at seq ($next)"
        }
        ["observation" "list"] => {
            refuse-foreign-flags $facade $sub $provided ["server" "run_id" "json"]
            let flags = {server: $server, run_id: $run_id, json: $json}
            let logged = ($s | update log ($s.log | append {argv: $argv, flags: $flags, stdin: $stdin_text}))
            save-state $p $logged
            let wanted_run = ($run_id | default '')
            let hits = ($logged | get -o observations | default [] | where {|o| $o.runId == $wanted_run} | each {|o| {seq: $o.seq, category: $o.category, text: $o.text}})
            {observations: $hits} | to json --raw | print
        }
        _ => {
            print -e $"fabro-work-shim: unsupported facade/subcommand '($facade) ($sub)'"
            exit 2
        }
    }
}
