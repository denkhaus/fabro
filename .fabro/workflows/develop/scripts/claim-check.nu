#!/usr/bin/env nu
# Claim-contract gate. Two deterministic checks stand between the
# planner's claim and the implementer:
#
# 1. Seed-id contract (fabro-c42f stopgap, 2026-09-18): the engine
#    resolves stdin_source="current_seed_id" BEFORE this script runs —
#    a missing key fails the node deterministically within seconds, so
#    a planner whose structured output validated routing but dropped
#    its context_updates parks the run EARLY instead of burning a
#    20-30 min implementer cycle. When the key exists, this arm only
#    asserts it is non-empty and well-formed (fabro- seed id prefix).
#
# 2. Seed-body path gate (fabro-4c81): every repo path the claimed
#    seed body cites as EXISTING evidence must resolve in the current
#    worktree before the claim is legal. The preflight flags rotted
#    citations advisory-only; the planner's mandated
#    `seeds update --description` correction was skipped at
#    reasoning_effort=low, so the wrong path stayed frozen in the seed
#    row and every later pass re-derived it. This gate gives that
#    mandate teeth: on a stale citation the node FAILS, the graph's
#    "Claim contract failed, re-plan" edge bounces the planner, and
#    the failure payload carries the stale path list plus the FULL
#    current body, so the correction is one seeds update call. The
#    blocking scope is deliberately narrow — only missing_file flags
#    for citations the body presents as existing (repo-rooted paths,
#    or any path:line anchor) that sit OUTSIDE a creation-intent
#    window (add/create/new/... right before the path — the body
#    naming the file the work will CREATE). out_of_range/mismatch
#    anchors and unrooted bare tokens stay advisory in the preflight:
#    they rot cited evidence, not the work routing. Fail-open on
#    every internal error (seeds CLI failure, JSON surprise): the
#    gate degrades to arm 1 only and never blocks a claim on tooling.

# Shared anchor/path verification (fabro-7daf anchors, fabro-9ec3 bare
# paths): extraction + existence/range checking, root/workflow-relative/
# workspace-member resolution. `source` resolves against THIS file's
# directory.
source anchor_check.nu

# Citations rooted at a known repo root are authoritative — the body
# presents them as existing repo state (the incident class: a body
# naming a step "of .fabro/workflows/develop/workflow.fabro" whose node
# is long gone). Unrooted slash-tokens stay advisory unless a line
# anchor pins them.
const ROOTED_PREFIXES = ['.fabro' 'lib' 'apps' 'scripts' 'docs' '.seeds' '.github' 'public']

# Creation-intent verbs: a path named immediately after one of these is
# the file the seed proposes to CREATE, not evidence it claims exists —
# blocking there would reject every forward-looking seed body. Plain
# const string (never interpolated: paren-heavy regex + $"..." would
# misparse).
const CREATE_INTENT = '(?i)\b(add|adds|adding|create|creates|created|creating|introduce|introduces|introduced|introducing|new|write|writes|writing|propose|proposes|proposed|proposing|sibling|missing|absent|nonexistent)\W*$'

def all-occurrences [hay: string, needle: string]: nothing -> list<int> {
    mut out = []
    mut rest = $hay
    mut offset = 0
    let n = ($needle | str length)
    let hlen = ($hay | str length)
    loop {
        let i = ($rest | str index-of $needle)
        if $i < 0 { break }
        $out = ($out | append ($offset + $i))
        let adv = $i + $n
        $offset = ($offset + $adv)
        if $offset >= $hlen { break }
        $rest = ($rest | str substring $adv..)
    }
    $out
}

def rooted? [p: string]: nothing -> bool {
    let seg = ($p | split row '/' | first)
    ($ROOTED_PREFIXES | any {|r| $r == $seg})
}

# True when EVERY occurrence of the path sits inside a creation-intent
# window (a creation verb within 64 chars before it). One non-creation
# occurrence is enough to treat the citation as existing-evidence.
def creation-named? [desc: string, needle: string]: nothing -> bool {
    let idxs = (all-occurrences $desc $needle)
    if ($idxs | is-empty) { return false }
    ($idxs | any {|i|
        if $i < 2 { false } else {
            let start = ([0 ($i - 64)] | math max)
            let window = ($desc | str substring $start..($i - 1))
            ($window =~ $CREATE_INTENT)
        }
    })
}

# Pure decision core: a `complete`-style record of `seeds show <id>
# --format json` plus the worktree root -> the claim-path verdict.
# Exported for claim-check-smoke.nu (closeout-smoke sourcing pattern).
#   outcome  succeeded | failed
#   degraded true  = internal/tooling failure, gate skipped (fail-open)
#   blocking missing_file citations the claim is illegal under
#   advisory rot flags that stay the preflight's business
#   body     the full current description, echoed only when blocking —
#            the raw material for the corrected seeds update call
export def claim-body-verdict [show: record, root: string] {
    if (($show | get -o exit_code | default 1) != 0) {
        return {outcome: "succeeded", degraded: true, note: "seeds show failed — path gate skipped (fail-open)", blocking: [], advisory: []}
    }
    let parsed = (try { $show.stdout | from json } catch { null })
    # nu's from json is lenient (a non-JSON word can come back as a
    # plain string) and describe names collection shapes in detail
    # ('record<...>') — gate on the shape prefix, not exact equality.
    if not (($parsed | describe) | str starts-with 'record') {
        return {outcome: "succeeded", degraded: true, note: "seeds show stdout was not a JSON object — path gate skipped (fail-open)", blocking: [], advisory: []}
    }
    if (($parsed | get -o success | default false) == false) {
        return {outcome: "succeeded", degraded: true, note: "seeds show answered success:false — path gate skipped (fail-open)", blocking: [], advisory: []}
    }
    let issue = ($parsed | get -o issue | default null)
    if $issue == null {
        return {outcome: "succeeded", degraded: true, note: "seed row carried no issue object — path gate skipped (fail-open)", blocking: [], advisory: []}
    }
    let desc = ($issue | get -o description | default "")
    if ($desc | is-empty) {
        return {outcome: "succeeded", degraded: false, note: null, blocking: [], advisory: []}
    }
    # URLs first: a `https://host.io:443/x` token parses as a path:line
    # anchor (`host.io` :443) and would false-block — strip them the way
    # extract-bare-paths already does.
    let clean = ($desc | str replace --all --regex '\S+://\S+' ' ')
    let flags = (
        (extract-anchors $clean | each {|a| check-anchor $a $root})
        | append (check-bare-paths $clean $root)
        | where {|f| $f.status != "ok"}
    )
    let blocking = ($flags | where {|f|
        (($f.status == "missing_file") and (($f.line != null) or (rooted? $f.path)) and (not (creation-named? $clean $f.path)))
    })
    let advisory = ($flags | where {|f| $f not-in $blocking })
    if ($blocking | is-not-empty) {
        {outcome: "failed", degraded: false, note: null, blocking: $blocking, advisory: $advisory, body: $desc}
    } else {
        {outcome: "succeeded", degraded: false, note: null, blocking: [], advisory: $advisory}
    }
}

def main []: nothing -> nothing {
    # Non-tty stdin: nu's `input` only works on a tty; the engine pipes the
    # context value, so read it through an external `cat` (closeout idiom).
    let raw = (cat | str join)
    let seed_id = ($raw | str trim)
    if ($seed_id | is-empty) {
        print -e "claim-check: stdin carried no seed id (stdin_source misconfigured?)"
        exit 1
    }
    if not ($seed_id | str starts-with "fabro-") {
        print -e $"claim-check: stdin value is not a seed id: ($seed_id)"
        exit 1
    }
    let show = (do { ^seeds show $seed_id --format json } | complete)
    let v = (claim-body-verdict $show ".")
    if $v.degraded {
        print -e $"claim-check: ($v.note)"
    } else if $v.outcome == "failed" {
        print -e $"claim-check: stale seed body ($seed_id) — cited paths do not resolve in the worktree:"
        for b in $v.blocking {
            let at = (if ($b | get -o line | default null) == null { "" } else { ":" + ($b.line | into string) })
            print -e $"  ($b.path)($at) [($b.status)]"
        }
        if ($v.advisory | is-not-empty) {
            print -e ('  advisory (non-blocking, preflight table): ' + ($v.advisory | to json --raw))
        }
        print -e ('claim-check: correct the tracker row FIRST — seeds update ' + $seed_id + ' --description "<full corrected body>" (re-emit the FULL body incl. the Basis: line, fix or drop the stale paths) — then route Seed claimed again. Current body follows.')
        print ($v.body)
        exit 1
    } else if ($v.advisory | is-not-empty) {
        print -e ('claim-check: advisory anchor flags (non-blocking): ' + ($v.advisory | to json --raw))
    }
    print $"claim-check: ok ($seed_id)"
}
