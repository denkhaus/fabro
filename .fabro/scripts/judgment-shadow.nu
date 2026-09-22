#!/usr/bin/env nu
# Judgment-shadow hook (fabro-8e13, ADR-0022 wave 1). SHARED by the loop
# workflows: each workflow.toml references this ONE copy from its
# [[run.hooks]] block — never duplicate per workflow.
#
# Fires on stage_complete (non-blocking, host-side: sandbox=false, so the
# script runs inside the server container, where the vendored nu and the
# server's env live). On evidence/reviewer/analyze completion it POSTs the
# stage's context_updates to the TypeSafe System One endpoint (model
# jev-latest; endpoint corrected from the seed's outdated OpenRouter
# premise, see docs.typesafe.ai/api) and appends ONE JSON line per call to
# .fabro/judgments/<run_id>.jsonl — schema fabro-judgment-v1.
#
# SHADOW ONLY: no engine decision consumes the stream. Thresholds and
# consumers wait for the evaluation report (ADR-0022 sequencing).
#
# Fail-open on EVERY path: missing key/context, timeout, 4xx/5xx, invalid
# response — a degraded line records the attempt, or the call is skipped;
# the run is never affected. The hook entry's engine timeout is the outer
# bound (a kill = skip, no line).
#
# Secrets: TYPESAFE_API_KEY comes from the server process env only
# (server-secrets-strategy); it never enters a run sandbox or a prompt.

const JUDGED_NODES = [evidence reviewer analyze]
const MODEL = "jev-latest"
const ENDPOINT_DEFAULT = "https://api.typesafe.ai/v1/systemone"

def main []: nothing -> nothing {
    let ctx_path = ($env.FABRO_HOOK_CONTEXT? | default "")
    if ($ctx_path | is-empty) {
        return
    }
    let ctx = (open $ctx_path)
    if (($ctx.event? | default "") != "stage_complete") {
        return
    }
    let node = ($ctx.node_id? | default "")
    if not ($node in $JUDGED_NODES) {
        return
    }
    let run_id = ($ctx.run_id? | default "")
    let key = ($env.TYPESAFE_API_KEY? | default "")
    let updates = ($ctx.context_updates? | default {})

    # Question shapes follow the System One API (docs.typesafe.ai/api):
    # a map of typed questions; choice questions carry the options as a
    # criteria map (option id -> meaning).
    let verdict = {
        type: "choice"
        instructions: "Adjudicate the reviewed change as a whole: does the evidence support the verdict the reviewer reached?"
        criteria: {
            approved: "The change satisfies its spec; approve."
            changes_requested: "The change has gaps; changes are requested."
            verification_blocked: "The evidence is insufficient to judge."
        }
    }
    let residue = (
        $updates.anomaly_files?
        | default []
        | each {|f| {
            $"residue:($f)": {
                type: "choice"
                instructions: $"A file changed by the diff is not named by the seed spec. Why is `($f)` in the diff?"
                criteria: {
                    residue: "Direct leftover of the implementation approach."
                    adjacent_repair: "Necessary repair adjacent to the spec."
                    scope_creep: "Beyond the spec; should be its own seed."
                    harmless_churn: "Cosmetic or mechanical churn."
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
        {degraded: "no_key"}
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
                if ($response | describe | str starts-with "string") {
                    $response | from json
                } else {
                    $response
                }
            )
            let latency = ((date now) - $started)
            {
                answers: ($response.answers? | default null)
                latency_ms: ($latency / 1ms | into int)
                cost_usd: ($response.usage?.cost_usd? | default null)
            }
        } catch {|err| {degraded: ($err.msg | str substring 0..120)}}
    }

    let visit = (
        if ($run_id | is-empty) { 0 } else {
            let file = $".fabro/judgments/($run_id).jsonl"
            if not ($file | path exists) { 1 } else {
                ((open --raw $file | lines | compact | length) + 1)
            }
        }
    )
    let entry = {
        schema: "fabro-judgment-v1"
        run_id: $run_id
        node: $node
        visit: $visit
        ts: (date now | format date "%+")
        model: $MODEL
        questions_hash: ($questions | to json -r | hash sha256)
        answers: ($line.answers? | default null)
        latency_ms: ($line.latency_ms? | default null)
        cost_usd: ($line.cost_usd? | default null)
        degraded: ($line.degraded? | default null)
    }
    if ($run_id | is-empty) {
        return
    }
    mkdir .fabro/judgments
    $"($entry | to json -r)\n" | save --append $".fabro/judgments/($run_id).jsonl"
}
