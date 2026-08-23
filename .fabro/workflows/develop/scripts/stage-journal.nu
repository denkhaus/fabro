#!/usr/bin/env nu
# Stage journal hook (ADR-0009 candidate, seed fabro-176b final design).
#
# Fires on stage_complete (non-blocking, sandbox). Writes ONE journal file
# per stage execution: .fabro/journal/<node>@<visit>.json
#
# Schema: fabro-journal-v1 — all fields required except data (object, may
# be empty). Provenance fields (node, visit, status, ts, run_id) come from
# the ENGINE hook context, never from agent claims. `data` is 1:1 the
# stage's declared journal payload from its context_updates (bridgeless:
# until HookContext carries context_updates — engine work item 2 — this
# hook reads nothing from agents and writes the envelope with empty data;
# consumers filter by their own schema inside data).
#
# File identity: <node>@<visit>.json is unique per execution; ts provides
# ordering; no global rank counter (deliberately — visit + ts suffice).
#
# Parallel stages: this hook fires once per SEQUENTIAL stage completion.
# Parallel branch targets do not run lifecycle hooks (engine limitation);
# the parent parallel node's completion fires this hook once with the
# bundled results available in context (staged for work item 2).

def main []: nothing -> nothing {
    let ctx_path = ($env.FABRO_HOOK_CONTEXT? | default "")
    if ($ctx_path | is-empty) {
        print "stage-journal: no FABRO_HOOK_CONTEXT — skipping"
        return
    }
    let ctx = (open $ctx_path)

    let node = ($ctx.node_id? | default "unknown")
    let event = ($ctx.event? | default "")
    if $event != "stage_complete" {
        return
    }

    # visit: hook context carries attempt/max_attempts, not the visit
    # counter; the visit is recovered from existing journal files for this
    # node (max existing visit + 1). First execution -> 1.
    let journal_dir = ".fabro/journal"
    if not ($journal_dir | path exists) {
        mkdir $journal_dir
    }
    let pattern = $"($journal_dir)/($node)@*.json"
    let existing = (glob $pattern | default [] | each {|f|
        ($f | path basename | parse --regex '@(?<v>[0-9]+)\.json$' | get 0?.v? | default "0" | into int)
    })
    let visit = (if ($existing | is-empty) { 1 } else { ($existing | math max) + 1 })

    let entry = {
        "$schema": "fabro-journal-v1",
        run_id: ($ctx.run_id? | default ""),
        node: $node,
        visit: $visit,
        status: ($ctx.status? | default ""),
        ts: (date now | format date "%Y-%m-%dT%H:%M:%SZ"),
        data: {}
    }

    let out = $"($journal_dir)/($node)@($visit).json"
    $entry | save --force $out
    print $"stage-journal: wrote ($out)"
}
