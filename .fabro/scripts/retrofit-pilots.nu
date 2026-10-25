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
    let pilots = [
        {id: 'fabro-d5f0', number: 6, labels: ['project:fabro' 'prio:p2' 'revision' 'seeds:fabro-d5f0' 'type:task']}
        {id: 'fabro-9337', number: 8, labels: ['project:fabro' 'prio:p2' 'residual' 'seeds:fabro-9337' 'type:task']}
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

        # body: through the deployed server's update endpoint
        let payload = ({itemKey: $"($repo)#($p.number)", body: $body} | to json)
        let patch = ($payload | do { http patch --headers [(auth-header)] $"($server)/api/v1/projects/($project)/work-items/update" -t application/json } | complete)
        print $"  body patch: exit ($patch.exit_code) ($patch.stdout | str substring 0..80)($patch.stderr | str substring 0..120)"
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
