#!/usr/bin/env nu
# judgment.nu — shared Jev (TypeSafe System One) judgment caller (ADR-0022).
#
# Vehicle for session-side judgments in the LOCAL session skills
# (iterate / integrate / merge-upstream; skill boundary 2026-09-16: never
# wired into fabro workflow prompts) and reusable by the engine-side
# judgment-shadow hook (fabro-8e13) later.
#
# ONE outbound HTTP call per invocation. FAIL-OPEN BY CONTRACT: every
# error path (missing key, timeout, 429/5xx, bad JSON, no input) still
# prints exactly one JSON record with degraded=true and exits 0 — a
# caller can never break because of this script.
#
# Usage:
#   nu .fabro/scripts/judgment.nu --request req.json
#   cat req.json | nu .fabro/scripts/judgment.nu --log-file ~/jd.jsonl \
#       --skill integrate --phase commit-triage --subject 'abc1234'
#
# Request shape ({state, questions}; the model is added here — callers
# never hardcode vendor versions):
#   {"state": ..., "questions": {"<id>": {"type":"noul|choice|score", ...}}}
#
# Output (stdout, one JSON record): answers + meta
#   {answers, model, latency_ms, cost_usd, status, degraded, error?}
#
# Log (--log-file): one fabro-judgment-v1 line per call, same fields as
# the engine stream plus session provenance (skill, phase, subject).
#
# Credentials: OPENROUTER_API_KEY from the environment ONLY. Never
# logged, never echoed, no fallback file location.

# One fabro-judgment-v1 log line for the given call outcome.
def judgment-log-line [
    model: string, questions_hash: string,
    skill: string, phase: string, subject: string,
    answers: any, latency_ms: float, cost_usd: any,
    degraded: bool, error: string,
]: nothing -> string {
    {
        schema: 'fabro-judgment-v1',
        ts: (date now | format date '%+'),
        skill: $skill,
        phase: $phase,
        subject: $subject,
        model: $model,
        questions_hash: $questions_hash,
        answers: $answers,
        latency_ms: $latency_ms,
        cost_usd: $cost_usd,
        degraded: $degraded,
        error: (if ($error | is-empty) { null } else { $error }),
    } | to json --raw
}

# Print the outcome record; optionally append the log line. Never fails.
def judgment-emit [record: any, log_file: string, log_line: string]: nothing -> nothing {
    if (not ($log_file | is-empty)) and (not ($log_line | is-empty)) {
        let parent = ($log_file | path dirname)
        if not ($parent | path exists) {
            mkdir $parent
        }
        ($"($log_line)\n") | save --append $log_file
    }
    $record | to json --raw | print
}

def main (
    --request: path                                  # JSON file with {state, questions}; omit to read stdin
    --model: string = 'typesafe/jev-1.13'            # version-pinned, never an alias (ADR-0022)
    --endpoint: string = 'https://openrouter.ai/api/alpha/decisions'
    --log-file: string = ''                          # append one fabro-judgment-v1 line per call
    --skill: string = ''                             # provenance: calling skill (integrate, iterate, ...)
    --phase: string = ''                             # provenance: phase inside the skill
    --subject: string = ''                           # provenance: judged object (commit sha, file, seed id)
    --max-time: duration = 30sec                     # outbound call budget
) {
    let start = (date now)
    let questions_hash = ''                          # filled once the request parsed

    def fail [error: string, latency_ms: float] {
        let line = (judgment-log-line $model $questions_hash $skill $phase $subject null $latency_ms null true $error)
        judgment-emit {answers: null, model: $model, latency_ms: $latency_ms, cost_usd: null, status: null, degraded: true, error: $error} $log_file $line
    }

    # --- credential (checked first: cheapest fail-open path) ----------------
    let key = $env.OPENROUTER_API_KEY?
    if ($key | is-empty) {
        fail 'OPENROUTER_API_KEY not set (fail-open: no judgment)' 0.0
        return
    }

    # --- gather the request -------------------------------------------------
    let req_text = if not ($request | is-empty) {
        try { open --raw $request } catch {|err|
            fail $"request unreadable: ($err.msg)" 0.0
            return
        }
    } else {
        # `nu script.nu` does NOT bridge process stdin into $in; read it
        # explicitly. Blocks on an interactive terminal until EOF — pipe or
        # use --request.
        try { open --raw /dev/stdin } catch {|err|
            fail $"stdin unreadable: ($err.msg)" 0.0
            return
        }
    }

    let req = try { $req_text | from json } catch {|err|
        fail $"request not valid JSON: ($err.msg)" 0.0
        return
    }
    # nu 0.115 `from json` is lenient with bare text (returns the string,
    # no error) — validate the shape before any cell-path access.
    if not (($req | describe) | str starts-with 'record') {
        fail 'request not a JSON object' 0.0
        return
    }
    if ($req.state? | is-empty) or ($req.questions? | is-empty) {
        fail 'request must carry non-empty state and questions' 0.0
        return
    }
    let questions_hash = ($req.questions | to json --raw | hash sha256)

    # --- call (allow-errors: HTTP error bodies stay parseable) --------------
    let headers = {Authorization: ('Bearer ' + $key)}
    let body = ({model: $model} | merge $req | to json --raw)
    let resp = try {
        http post -r -e --max-time $max_time --headers $headers --content-type application/json $endpoint $body
    } catch {|err|
        fail $"call failed: ($err.msg | str substring 0..300)" (((date now) - $start) / 1ms)
        return
    }

    let parsed = try { $resp | from json } catch {|err|
        fail $"response not JSON: ($err.msg | str substring 0..300)" (((date now) - $start) / 1ms)
        return
    }
    if ($parsed.answers? | is-empty) {
        let detail = ($parsed.error?.message? | default ($parsed | to json --raw) | str substring 0..300)
        fail $"endpoint error: ($detail)" (((date now) - $start) / 1ms)
        return
    }

    let latency_ms = (((date now) - $start) / 1ms)
    let line = (judgment-log-line $model $questions_hash $skill $phase $subject $parsed.answers $latency_ms $parsed.usage?.cost? false '')
    judgment-emit {answers: $parsed.answers, model: ($parsed.model? | default $model), latency_ms: $latency_ms, cost_usd: $parsed.usage?.cost?, status: 'ok', degraded: false} $log_file $line
}
