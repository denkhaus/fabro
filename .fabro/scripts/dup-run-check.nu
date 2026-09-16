#!/usr/bin/env nu
# Deterministic duplicate-run preflight (fabro-8c75): does a merged
# IMPLEMENTATION of the given seed id(s) already exist on the merge-target
# branch? Replaces the any-commit grep + LLM-judgment clause in the
# implementer preflight and the revisor duplicate-run check (Step 3.5).
#
# Usage:
#   nu .fabro/scripts/dup-run-check.nu <seed-id> [<seed-id>...] [--base <ref>]
#   (default base: origin/denkhaus — the FACTS merge-target branch)
#
# Mechanical classification, no LLM judgment:
#   - tracker-authoritative: `sd show <id>` status closed -> duplicate.
#   - history: commits on the base ref referencing the seed id, restricted
#     to LANDED-PR commits — true merge commits (2-parent) OR squash-merge
#     subjects ending in `(#<n>)`.
#   - a landed-PR commit whose subject names a revisor pass only FILED the
#     seed -> filed-only, NOT a duplicate.
#   - non-PR commits referencing the id (seed-sync/tracker churn) are
#     reported as `other` — informational, never a duplicate.
#
# Output: one JSON object per seed id (JSONL stream on stdout). Verdicts:
#   duplicate | clean | degraded
# Exit 0 unless the invocation itself is wrong (then 2).

def parse-log [text: string] {
    $text
    | lines
    | compact
    | each {|l|
        let p = ($l | parse --regex '^(?<sha>[0-9a-f]{7,40}) (?<subject>.*)$')
        if ($p | is-empty) { null } else { $p | first }
    }
    | compact
}

def git-log-matching [base, id, extra] {
    let r = (do { ^git log ...$extra --format="%H %s" --grep $id -n 200 $base } | complete)
    if $r.exit_code != 0 {
        return {rows: [], error: $r.stderr}
    }
    {rows: (parse-log $r.stdout), error: null}
}

def classify-filed [rows] {
    $rows | each {|r|
        {sha: $r.sha, subject: $r.subject, filed_only: ($r.subject =~ '(?i)revisor (pass|:)')}
    }
}

def main [...ids: string, --base: string = "origin/denkhaus"] {
    if ($ids | is-empty) {
        print -e "dup-run-check: no seed id given — usage: nu .fabro/scripts/dup-run-check.nu <seed-id>... [--base <ref>]"
        exit 2
    }
    let remote = ($base | split row "/" | first)
    let branch = ($base | split row "/" | skip 1 | str join "/")
    let fetch = (do { ^git fetch $remote $branch } | complete)

    for id in $ids {
        mut tracker_status = "unknown"
        mut tracker_note = ""
        let sd_ok = ((which sd | length) > 0)
        if $sd_ok {
            let s = (do { sd show $id --format json } | complete)
            if $s.exit_code == 0 {
                $tracker_status = ($s.stdout | from json | get issue.status? | default "unknown")
            } else {
                $tracker_note = "sd show failed — tracker check skipped"
            }
        } else {
            $tracker_note = "sd absent — tracker check skipped"
        }

        if $fetch.exit_code != 0 {
            {seed: $id, verdict: "degraded", reason: "fetch failed", detail: $fetch.stderr} | to json --raw | print
            continue
        }

        let merges = (git-log-matching $base $id [--merges]).rows
        let all_no_merge = (git-log-matching $base $id [--no-merges]).rows
        let squashes = ($all_no_merge | where {|r| ($r.subject =~ '\(#\d+\)$')})
        let landed = (classify-filed ($merges | append $squashes))
        let implementations = ($landed | where {|r| not $r.filed_only})
        let filed_only = ($landed | where {|r| $r.filed_only})
        let other = ($all_no_merge | where {|r| not ($r.subject =~ '\(#\d+\)$')})

        let verdict = (if $tracker_status == "closed" or ($implementations | length) > 0 {
            "duplicate"
        } else {
            "clean"
        })
        {seed: $id,
         verdict: $verdict,
         tracker_status: $tracker_status,
         tracker_note: (if ($tracker_note | is-empty) { null } else { $tracker_note }),
         implementation_matches: $implementations,
         filed_only_matches: ($filed_only | length),
         other_refs: ($other | length)} | to json --raw | print
    }
}
