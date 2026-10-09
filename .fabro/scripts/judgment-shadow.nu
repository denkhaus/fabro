#!/usr/bin/env nu
# Judgment-shadow hook (fabro-8e13, ADR-0022 wave 1; fabro-d367 transport
# fix). SHARED by the loop workflows: each workflow.toml carries THIS ONE
# copy — byte-identical — inside its [[run.hooks]] script string (a quoted
# heredoc piped through `nu -c`), never duplicated by hand; the
# judgment-shadow sync battery (battery-runner registry) REDs on drift.
#
# WHY THE HEREDOC TRANSPORT (fabro-d367): the hook is host-side
# (sandbox=false) so OPENROUTER_API_KEY stays in the server env and never
# enters a run sandbox — but a host-side hook runs with `sh -c` and NO
# workspace cwd whenever the run's sandbox does not share the host
# filesystem (the Docker provider, i.e. the packaged server container,
# where the repo's files do not exist at all). A repo-relative
# `nu .fabro/scripts/judgment-shadow.nu` there exits 1 on EVERY stage —
# file not found, before any node filtering — silently, because the hook
# is non-blocking. Carrying the script inside the command is the only
# transport that reaches the server container; the file stays the source
# of truth for it.
#
# Fires on stage_complete. On evidence/reviewer/analyze completion it
# POSTs the stage's context_updates to OpenRouter's System One endpoint
# (model jev-latest; operator repoint 2026-09-24: there is no
# TYPESAFE_API_KEY — jev runs through OpenRouter with the
# OPENROUTER_API_KEY that already sits in the mirtuell server env.
# OpenRouter maps bare System One model ids like jev-latest onto the
# typesafe/ namespace; usage reports cost as usage.cost, not cost_usd;
# see openrouter.ai/docs — Submit a System One request) and appends ONE
# JSON line per call to a fabro-judgment-v1 stream.
#
# STREAM ANCHOR (fabro-d367): `.fabro/judgments/<run_id>.jsonl` when the
# cwd is a workspace (the file rides the run branch — host-provider and
# dev runs), else `$FABRO_STORAGE_DIR/judgments/<run_id>.jsonl` (the
# packaged server container's persistent volume), else stdout only. With
# no anchor reachable the line still prints, so the attempt is visible.
#
# SHADOW ONLY: no engine decision consumes the stream. Thresholds and
# consumers wait for the evaluation report (ADR-0022 sequencing).
#
# Fail-open on EVERY path (fabro-d367: LOUD fail-open): missing
# key/context, timeout, 4xx/5xx, invalid response, script-internal errors
# — a degraded line records the attempt wherever an anchor exists, a
# one-line status always prints to stdout, and the exit code is ALWAYS 0
# so a non-blocking hook never reports a silent code-1 'executed' again.
# The engine hook timeout remains the outer bound (a kill = skip, no
# line).
#
# Secrets: OPENROUTER_API_KEY comes from the server process env only
# (server-secrets-strategy); it never enters a run sandbox or a prompt.
#
# EMBEDDING CONTRACT: this file is embedded verbatim inside the four
# workflow.toml hook entries, so it must contain no line equal to the
# heredoc delimiter (JSEOF) and no run of three single quotes. The entry
# explicit call at the bottom (not `def main`) so the same bytes run
# identically as a script file (nu auto-calls only `main`) and through
# `nu -c` (which auto-calls nothing).

const JUDGED_NODES = [evidence reviewer analyze]
const MODEL = "jev-latest"
const ENDPOINT_DEFAULT = "https://openrouter.ai/api/v1/systemone"

# Where this call's line lands: the workspace stream when the cwd is a
# checkout, the server storage volume otherwise. null = stdout only.
def stream_path [run_id: string] {
    if ($run_id | is-empty) { return null }
    if ('.fabro' | path type) == 'dir' {
        return $".fabro/judgments/($run_id).jsonl"
    }
    let storage = ($env.FABRO_STORAGE_DIR? | default '')
    if ($storage | is-empty) { return null }
    $"($storage | str trim -r -c '/')/judgments/($run_id).jsonl"
}

def append_line [path: string, entry: record]: nothing -> nothing {
    let dir = ($path | path dirname)
    if ($dir != '.') { mkdir $dir }
    $"($entry | to json -r)\n" | save --append $path
}

def judgment_shadow_hook []: nothing -> nothing {
    let ctx_path = ($env.FABRO_HOOK_CONTEXT? | default '')
    if ($ctx_path | is-empty) {
        print 'judgment-shadow: no FABRO_HOOK_CONTEXT — skipped'
        return
    }
    # fabro-d367: an unreadable/invalid context file must degrade, not
    # exit 1 (the original silent every-stage failure class).
    let ctx = (
        try { open $ctx_path } catch {|err|
            print $"judgment-shadow: degraded (context_open: ($err.msg | str substring 0..120))"
            return
        }
    )
    # A non-JSON context file opens as a plain string; only a record can
    # carry the fields this hook filters on.
    if not (($ctx | describe) | str starts-with 'record') {
        print 'judgment-shadow: degraded (context_open: context is not a record)'
        return
    }
    if (($ctx.event? | default '') != 'stage_complete') { return }
    let node = ($ctx.node_id? | default '')
    if not ($node in $JUDGED_NODES) { return }
    let run_id = ($ctx.run_id? | default '')
    let key = ($env.OPENROUTER_API_KEY? | default '')
    let updates = ($ctx.context_updates? | default {})

    # Question shapes follow the System One API (same schema on both
    # surfaces; OpenRouter reference: Submit a System One request):
    # a map of typed questions; choice questions carry the options as a
    # criteria map (option id -> meaning).
    let verdict = {
        type: 'choice'
        instructions: 'Adjudicate the reviewed change as a whole: does the evidence support the verdict the reviewer reached?'
        criteria: {
            approved: 'The change satisfies its spec; approve.'
            changes_requested: 'The change has gaps; changes are requested.'
            verification_blocked: 'The evidence is insufficient to judge.'
        }
    }
    let residue = (
        $updates.anomaly_files?
        | default []
        | each {|f| {
            $"residue:($f)": {
                type: 'choice'
                instructions: $"A file changed by the diff is not named by the seed spec. Why is `($f)` in the diff?"
                criteria: {
                    residue: 'Direct leftover of the implementation approach.'
                    adjacent_repair: 'Necessary repair adjacent to the spec.'
                    scope_creep: 'Beyond the spec; should be its own seed.'
                    harmless_churn: 'Cosmetic or mechanical churn.'
                }
            }
        } }
        | reduce -f {} {|it, acc| $acc | merge $it }
    )
    let questions = ($residue | merge {verdict_pre_screen: $verdict})

    # Ops seam: an explicit endpoint override (self-hosted proxies,
    # scripted twin tests); the production default stays the pinned const.
    let endpoint = ($env.JUDGMENT_SHADOW_ENDPOINT? | default $ENDPOINT_DEFAULT)
    let started = (date now)
    let line = if ($key | is-empty) {
        {degraded: 'no_key'}
    } else {
        try {
            let response = (
                http post
                    --headers {Authorization: $"Bearer ($key)"}
                    --content-type application/json
                    $endpoint
                    {
                        model: $MODEL
                        state: {run_id: $run_id, node: $node, context_updates: $updates}
                        questions: $questions
                    }
            )
            # nu hands back a parsed record for JSON responses and a
            # plain string otherwise; normalize before cell-path access.
            let response = (
                if ($response | describe | str starts-with 'string') {
                    $response | from json
                } else {
                    $response
                }
            )
            let latency = ((date now) - $started)
            {
                answers: ($response.answers? | default null)
                latency_ms: ($latency / 1ms | into int)
                # OpenRouter's System One response reports usage.cost
                # (the typesafe.ai surface called it cost_usd).
                cost_usd: ($response.usage?.cost? | default null)
            }
        } catch {|err| {degraded: ($err.msg | str substring 0..120)}}
    }

    let visit = (
        if ($run_id | is-empty) { 0 } else {
            let file = (stream_path $run_id)
            if ($file == null) { 1 } else {
                if not ($file | path exists) { 1 } else {
                    ((open --raw $file | lines | compact | length) + 1)
                }
            }
        }
    )
    let entry = {
        schema: 'fabro-judgment-v1'
        run_id: $run_id
        node: $node
        visit: $visit
        ts: (date now | format date '%+')
        model: $MODEL
        questions_hash: ($questions | to json -r | hash sha256)
        answers: ($line.answers? | default null)
        latency_ms: ($line.latency_ms? | default null)
        cost_usd: ($line.cost_usd? | default null)
        degraded: ($line.degraded? | default null)
    }
    let file = (stream_path $run_id)
    if ($file == null) {
        # No anchor reachable: keep the attempt visible on stdout —
        # never a silent exit, never an unanchored write.
        print $"judgment-shadow: no stream anchor — ($entry | to json -r)"
        return
    }
    # fabro-d367: the append itself is guarded too — a read-only anchor
    # degrades loudly instead of exiting 1.
    try {
        append_line $file $entry
        print $"judgment-shadow: ($node) recorded \(degraded: ($entry.degraded? | default 'null'))"
    } catch {|err|
        print $"judgment-shadow: degraded (append: ($err.msg | str substring 0..120))"
    }
}

judgment_shadow_hook
