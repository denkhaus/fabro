#!/usr/bin/env nu
# fabro-work-shim (denkhaus/work#990): the host-side channel shim for
# batteries — a state-file fake for the `fabro work` facade so the lane
# scripts' channel contracts are pinnable without a server. The real
# channel exists only inside run sandboxes (FABRO_RUN_CHANNEL_SOCK is
# the run's mount); batteries point FABRO_BIN at THIS script and
# FABRO_WORK_SHIM_STATE at a JSON state file. Every invocation is
# appended to state.log (argv + captured flags + stdin) so a smoke can
# assert the exact call sequence, and every mutation lands in the state
# file so a smoke can assert the durable outcome.
#
# State shape (camelCase, mirroring the CLI's OpenAPI wire form):
#   {items: [{itemKey,title,body,state,labels}],
#    claims: [{claimId,itemKey,runId,lane,state}],
#    nextKey: <int>,
#    log: [{argv,flags,stdin}]}
#
# The shim answers the verbs the lane scripts use — show, claims, list,
# create, close. main's signature declares every flag those verbs take
# (nu parses them out of the command line; $argv keeps the subcommand
# chain and the positionals). It is a TEST ASSET: nothing in production
# may resolve the fabro binary here.

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

def main [
    --server: string,             # ignored (the transport is faked)
    --project: string,            # ignored (single-project state)
    --query: string,
    --title: string,
    --type: string,
    --reason: string,
    --body-file: string,          # kebab declaration, binds $body_file
    --label: string,              # comma-joined (the CLI's documented form)
    --priority: string,
    --lane: string,
    --limit: string,
    --json,
    ...argv: string
]: nothing -> nothing {
    let p = (state-path)
    let stdin_text = (cat | str join)
    let s = (load-state $p)
    let flags = {server: $server, project: $project, query: $query, title: $title, itemType: $type, reason: $reason, bodyFile: $body_file, label: $label, priority: $priority, lane: $lane, limit: $limit, json: $json}
    let logged = ($s | update log ($s.log | append {argv: $argv, flags: $flags, stdin: $stdin_text}))
    save-state $p $logged

    # $argv keeps the subcommand chain and the selector positional.
    if (($argv | length) < 2) or ($argv.0 != "work") {
        print -e $"fabro-work-shim: expected 'work <subcommand>', got: ($argv | str join ' ')"
        exit 2
    }
    let sub = $argv.1
    let positional = ($argv | skip 2 | get -o 0 | default '')

    match $sub {
        "show" => {
            let item = ($logged.items | where {|it| $it.itemKey == $positional} | get -o 0 | default null)
            if $item == null {
                print -e $"no item '($positional)' in the shim state"
                exit 1
            }
            if $json { $item | to json --raw | print } else { print $"($item.itemKey): ($item.title)" }
        }
        "claims" => {
            $logged.claims | to json --raw | print
        }
        "list" => {
            let hits = ($logged.items | where {|it|
                ($it.state == "open") and (
                    ($query | is-empty) or ($it.title | str contains $query) or ($it.body | str contains $query)
                )
            })
            $hits | to json --raw | print
        }
        "create" => {
            let n = ($logged | get -o nextKey | default 9000)
            let key = $"denkhaus/work#($n)"
            let labels = (if ($label | is-empty) { [] } else { $label | split row "," })
            let item = {itemKey: $key, title: ($title | default ''), body: $stdin_text, state: "open", labels: $labels}
            save-state $p ($logged | update items ($logged.items | append $item) | update nextKey ($n + 1))
            print $key
        }
        "close" => {
            let item = ($logged.items | where {|it| $it.itemKey == $positional and $it.state == "open"} | get -o 0 | default null)
            if $item == null {
                print -e $"no open item '($positional)' to close in the shim state"
                exit 1
            }
            let closed = ($logged.items | each {|it| if $it.itemKey == $positional { $it | update state "closed" } else { $it } })
            save-state $p ($logged | update items $closed)
            print $"Closed ($positional)."
        }
        _ => {
            print -e $"fabro-work-shim: unsupported subcommand '($sub)'"
            exit 2
        }
    }
}
