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
#    `seeds update --set-description` correction was skipped at
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
#
# 3. Claim-time line-tip guard (fabro-006d): the run pins its checkout
#    at create time; an auto-merge landing seconds later leaves this
#    stage scanning a base where ALREADY-LANDED work is invisible — the
#    TOCTOU that re-implemented fabro-828c as conflicting PR #412. The
#    guard fetches the line, and when the base is behind it scans
#    base..line for (a) the claimed seed id in commit bodies and (b)
#    the seed body's rooted repo paths in the touched files. A hit
#    bounces the planner to re-verify instead of implementing. Fail-open
#    on every git error (offline sandbox, missing ref): the arm skips.

# Shared anchor/path verification (fabro-7daf anchors, fabro-9ec3 bare
# paths): extraction + existence/range checking, root/workflow-relative/
# workspace-member resolution. `source` resolves against THIS file's
# directory.
source anchor_check.nu

# Spec-named loop-asset paths — the SAME shared extraction the
# tester's seed-spec-paths tier uses (`spec-named-loop-paths` in
# .fabro/scripts/lib/spec_paths.nu, sourced there by
# scripts/qualitygate.nu), sourced, never re-implemented. This script
# already parses the claimed seed's body, so it also emits that path
# list into stage output: the implementer's cheap pre-tester
# self-check diffs the list against `git diff --name-only` and a
# spec-paths miss dies at the implementer instead of bouncing off the
# tester battery. `source` resolves against THIS file's directory;
# the emission itself is fail-open ([] on any internal error, lane
# convention — a broken probe never blocks a claim).
source ../../../scripts/lib/spec_paths.nu

# Citations rooted at a known repo root are authoritative — the body
# presents them as existing repo state (the incident class: a body
# naming a step "of .fabro/workflows/develop/workflow.fabro" whose node
# is long gone). Unrooted slash-tokens stay advisory unless a line
# anchor pins them.
const ROOTED_PREFIXES = ['.fabro' 'lib' 'apps' 'scripts' 'docs' '.seeds' '.github' 'public']

def rooted? [p: string]: nothing -> bool {
    let seg = ($p | split row '/' | first)
    ($ROOTED_PREFIXES | any {|r| $r == $seg})
}

# The creation-intent window (CREATE_INTENT, all-occurrences,
# creation-named?) moved to anchor_check.nu as the shared seam: the
# preflight's bare-path arm applies the same window (fabro-f2eb), so a
# proposed-file citation is false-flagged nowhere.

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

# Pure decision core of arm 3: the run base sha, the line tip, the
# `git log --format=%B --name-only base..tip` text, the claimed seed id,
# and the seed body -> the line-tip verdict. Exported for the smoke
# battery; the git plumbing lives in `main` and fails open.
#
#   outcome  succeeded | failed
#   skipped  base == tip (nothing landed since the pin) or the arm was
#            not given a range to scan (fail-open)
#   evidence what matched: {seed_in: <line>} when a commit body names
#            the seed id, {path_in: <path>} for each rooted body path a
#            landed commit touched
export def line-tip-verdict [
    base: string, tip: string, range_log: string, seed_id: string, body: string
] {
    if ($base | is-empty) or ($tip | is-empty) or ($base == $tip) {
        return {outcome: "succeeded", skipped: true, note: null, evidence: []}
    }
    if ($range_log | is-empty) {
        return {outcome: "succeeded", skipped: true, note: "no range to scan — arm skipped (fail-open)", evidence: []}
    }
    let clean = ($body | str replace --all --regex '\S+://\S+' ' ')
    let cited = (extract-bare-paths $clean | where {|p| rooted? $p})
    mut evidence = []
    for line in ($range_log | lines) {
        if ($line | str contains $seed_id) {
            $evidence = ($evidence | append {seed_in: $line})
        }
        if ($line | str trim) in $cited {
            $evidence = ($evidence | append {path_in: ($line | str trim)})
        }
    }
    if ($evidence | is-not-empty) {
        {outcome: "failed", skipped: false, note: null, evidence: $evidence}
    } else {
        {outcome: "succeeded", skipped: false, note: null, evidence: []}
    }
}

const LINE_REF = "origin/denkhaus"

# The git plumbing of arm 3 (fail-open on every error): fetch the line,
# resolve base and tip, scan the range, and hand the pure core its inputs.
def line-tip-guard [seed_id: string, body: string] {
    let fetched = (do { ^git fetch origin denkhaus } | complete)
    if $fetched.exit_code != 0 {
        return {outcome: "succeeded", skipped: true, note: "git fetch failed — line-tip arm skipped (fail-open)"}
    }
    let base = (do { ^git rev-parse HEAD } | complete)
    let tip = (do { ^git rev-parse $LINE_REF } | complete)
    if ($base.exit_code != 0) or ($tip.exit_code != 0) {
        return {outcome: "succeeded", skipped: true, note: "rev-parse failed — line-tip arm skipped (fail-open)"}
    }
    let base_sha = ($base.stdout | str trim)
    let tip_sha = ($tip.stdout | str trim)
    let range = (do { ^git log --format=%B --name-only $"($base_sha)..($tip_sha)" } | complete)
    if $range.exit_code != 0 {
        return {outcome: "succeeded", skipped: true, note: "git log range failed — line-tip arm skipped (fail-open)"}
    }
    line-tip-verdict $base_sha $tip_sha $range.stdout $seed_id $body
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
    let guard_body = (
        try { $show.stdout | from json | get -o issue | default {} | get -o description | default '' } catch { '' }
    )
    let tip = (line-tip-guard $seed_id $guard_body)
    if $tip.outcome == "failed" {
        print -e $"claim-check: base stale — work landed on the line since this run's pin. Re-verify the claim before implementing \($seed_id\):"
        for e in $tip.evidence {
            print -e $"  ($e | to json --raw)"
        }
        print -e "claim-check: route Seed claimed again after re-verifying against the current line tip."
        exit 1
    } else if ($tip | get -o skipped | default false) and ($tip | get -o note | default null) != null {
        print -e $"claim-check: ($tip.note)"
    }
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
        print -e ('claim-check: correct the tracker row FIRST — seeds update ' + $seed_id + ' --set-description "<full corrected body>" (re-emit the FULL body incl. the Basis: line, fix or drop the stale paths) — then route Seed claimed again. Current body follows.')
        print ($v.body)
        exit 1
    } else if ($v.advisory | is-not-empty) {
        print -e ('claim-check: advisory anchor flags (non-blocking): ' + ($v.advisory | to json --raw))
    }
    print $"claim-check: ok ($seed_id)"
    # Spec-paths emission (fail-open): the implementer's self-check
    # input. Even a degraded path gate (empty body) emits `[]` — the
    # downstream diff is then vacuously green.
    let spec_paths = (do { spec-named-loop-paths $guard_body } | default [])
    print $"claim-check: spec-paths ($spec_paths | to json --raw)"
}
