#!/usr/bin/env nu
# retrofit-pilots.nu — ONE-OFF (fabro-4fb4, real-import stage 3): bring the
# two hand-migrated pilots onto the canonical importer form so the gateway
# diff covers all 974 items uniformly.
#   fabro-d5f0  -> denkhaus/work#6   fabro-9337 -> denkhaus/work#8
# Steps per pilot: ensure labels exist, set the exact label set (gh — the
# server API has no label-write path), then PATCH the canonical body
# through the DEPLOYED server's update endpoint (the write path stays the
# server's). Run AFTER the real import (prio/type labels then exist) and
# BEFORE verify.

def main [
    --server: string = 'https://mirtuell.net'
    --project: string = 'fabro'
    --repo: string = 'denkhaus/work'
    --apply: int = 0                  # 0 = show plan only, 1 = write
]: nothing -> nothing {
    # Refresh the rotating token through an authenticated CLI call first:
    # the raw token in auth.json may be expired (the CLI rotates it; a bare
    # read races the rotation clock).
    let _ = (do { ^'/home/denkhaus/dev/fabro/target/debug/fabro' work --server $server claims --project $project } | complete)
    let pilots = [
        {id: 'fabro-d5f0', number: 6, title: 'spec_paths extractor: expand {a,b,c} brace groups before the path filter', labels: ['project:fabro' 'prio:p2' 'revision' 'seeds:fabro-d5f0' 'type:task']}
        {id: 'fabro-9337', number: 8, title: "Deferred follow-up from fabro-4653: extend prompt-lint's vendor-CLI check (check 9) scope from develop-onl", labels: ['project:fabro' 'prio:p2' 'residual' 'seeds:fabro-9337' 'type:task']}
    ]
    let token = (auth-token)
    for $p in $pilots {
        let body_path = $"tmp/cutover/canonical-($p.id).md"
        if not ($body_path | path exists) { print -e $"retrofit: ($body_path) missing — run the importer canonical mode first"; exit 1 }
        let body = (open --raw $body_path)
        print $"retrofit ($p.id) -> ($repo)#($p.number): labels ($p.labels | str join ',') + canonical body"
        if $apply == 0 { continue }

        # labels: ensure each exists, then set the exact set (PUT replaces)
        for $label in $p.labels {
            let _ = (do { gh api --method POST $"repos/($repo)/labels" -f name=$label -f color=ededed } | complete)
        }
        let set_labels = ({labels: $p.labels} | to json)
        let lab = ($set_labels | do { gh api --method PUT $"repos/($repo)/issues/($p.number)/labels" --input - } | complete)
        print $"  labels: exit ($lab.exit_code) ($lab.stderr | str trim | str substring 0..120)"

        # body + title: through the deployed server's update endpoint (POST
        # — the route is work-items/update via post, not PATCH). The title
        # is part of the canonical form: the pilot's hand title must not
        # survive as a second naming (verify diff found exactly that on
        # fabro-9337).
        let payload = ({itemKey: $"($repo)#($p.number)", title: $p.title, body: $body} | to json)
        let url = $"($server)/api/v1/projects/($project)/work-items/update"
        let patch = (try {
            let r = ($payload | http post --headers (auth-header) --full $url -t application/json)
            {ok: ($r.status == 200), detail: $"status ($r.status): ($r.body | to json -r | str substring 0..100)"}
        } catch {|err|
            {ok: false, detail: ($err.msg | default ($err | to json -r) | str substring 0..160)}
        })
        print $"  body update: ($patch.detail)"
    }
    if $apply == 0 { print 'retrofit: plan shown; rerun with --apply 1 to write' }
}

def auth-token []: nothing -> string {
    let raw = (open --raw ~/.fabro/auth.json | from json)
    let servers = ($raw | get -o servers | default {})
    let entry = ($servers | get -o $"https://mirtuell.net" | default {})
    if ($entry | is-empty) { print -e 'retrofit: no mirtuell entry in ~/.fabro/auth.json'; exit 1 }
    $entry | get -o token | default ($entry | get -o access_token | default '')
}

def auth-header []: nothing -> list<string> {
    [Authorization $"Bearer (auth-token)"]
}
