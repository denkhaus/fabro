#!/usr/bin/env nu
# Deterministic duplicate-run preflight (fabro-8c75): does a merged
# IMPLEMENTATION of the given seed id(s) already exist on the merge-target
# branch? Replaces the any-commit grep + LLM-judgment clause in the
# implementer preflight and the revisor duplicate-run check (Step 3.5).
#
# Usage:
#   nu .fabro/scripts/dup-run-check.nu <seed-id> [<seed-id>...] [--base <ref>] [--self <run-id>]
#   (default base: origin/denkhaus — the FACTS merge-target branch)
#
# Mechanical classification, no LLM judgment:
#   - tracker-authoritative: `sd show <id>` status closed -> duplicate,
#     UNLESS the closing evidence classifies as a self-closure (--self).
#   - history: commits on the base ref referencing the seed id, restricted
#     to LANDED-PR commits — true merge commits (2-parent) OR squash-merge
#     subjects ending in `(#<n>)`.
#   - a landed-PR commit whose subject names a revisor pass only FILED the
#     seed -> filed-only, NOT a duplicate.
#   - non-PR commits referencing the id (seed-sync/tracker churn) are
#     reported as `other` — informational, never a duplicate.
#
# Closure identity (fabro-e6a0): every implementation match carries a
# `closure` field derived from the commit's `Fabro-Run:` trailer —
#   self    the trailer names the invoking run (--self <run-id>)
#   foreign anything else (no trailer, or a different run); the raw
#           trailer run-id ships as `trailer_run` for consumers
# Self matches never drive a `duplicate` verdict: with --self supplied they
# auto-downgrade the verdict to `clean` with a `closure_note`. Status-
# independent: an in_progress seed whose own landed PR matches is a
# self-closure, not a duplicate. When the tracker says closed but no
# implementation match exists, the closing commit is resolved (last commit
# touching .seeds referencing the id, else any body-matching commit) and
# reported as `closing_evidence` with the same closure classification.
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

# fabro-a32f: the filed-only marker covers every revisor-line commit
# convention observed on the merge target, not just `Revisor pass:` —
# `Revise run <id>; file fabro-xxxx (#n)` (e.g. a7ee183 filing
# fabro-ea41/fabro-7aac via PR #206) and `file N ... seeds` subjects
# MERELY FILE the named seeds and must never count as landed
# implementations. Conservative by design: when a revisor commit both
# implements and files, it is classified filed-only (a false negative
# routes the planner normally; a false positive would mechanically
# close live work in the pre-planner preflight).
def classify-filed [rows] {
    $rows | each {|r|
        {sha: $r.sha, subject: $r.subject, filed_only: ($r.subject =~ '(?i)(revisor (pass|:))|(\brevise run\b)|(\bfile\s+\d+\s+[^;()]*seeds?\b)|(;\s*file\s+fabro-[0-9a-z]+)')}
    }
}

# Fabro-Run trailer of one commit; null when the commit has none.
def trailer-run [sha: string] {
    let r = (do { ^git log -1 --format=%B $sha } | complete)
    if $r.exit_code != 0 {
        return null
    }
    let m = ($r.stdout | parse --regex '(?m)^[ \t]*Fabro-Run:[ \t]*(?<run>\S+)')
    if ($m | is-empty) { null } else { $m | last | get run }
}

# closure identity vs the invoking run: self only when the trailer names it.
def classify-closure [tr, self_id] {
    if ($tr != null and $self_id != null and $tr == $self_id) { "self" } else { "foreign" }
}

# Add trailer identity + closure to each implementation match.
def with-closure [rows, self_id] {
    $rows | each {|r|
        let tr = (trailer-run $r.sha)
        {sha: $r.sha,
         subject: $r.subject,
         filed_only: $r.filed_only,
         trailer_run: $tr,
         closure: (classify-closure $tr $self_id)}
    }
}

# Last commit whose .seeds patch ADDS the closed status for this id — the
# mechanical form of "tracker closedAt evidence" (sd writes one compact JSON
# line per issue; the closing edit adds '<id> ... "status":"closed"').
def closing-seeds-commit [base, id] {
    let r = (do { ^git log --format="%H %s" -n 50 $base -- .seeds } | complete)
    if $r.exit_code != 0 {
        return null
    }
    for row in (parse-log $r.stdout) {
        let p = (do { ^git show $row.sha -- .seeds } | complete)
        if $p.exit_code == 0 {
            let added = ($p.stdout | lines | where {|l|
                ($l | str starts-with "+") and ($l | str contains $id) and ($l | str contains '"status":"closed"')})
            if (not ($added | is-empty)) {
                return $row
            }
        }
    }
    null
}

# Inherit a Fabro-Run trailer for a trailer-less seeds-close commit from the
# adjacent landed-PR commit: the seeds-close commit's first parent when it
# carries a trailer, else the nearest preceding landed-PR commit (true merge
# or squash subject `(#<n>)`) that carries one. sd stamps no trailer on its
# closing sync commit, but that sync sits directly on the closing run's
# landed PR (fabro-cf2a). Null when nothing adjacent carries a trailer.
def inherited-trailer [base, sha] {
    mut cands = []
    let fp = (do { ^git rev-parse $"($sha)^" } | complete)
    if $fp.exit_code == 0 and (not ($fp.stdout | str trim | is-empty)) {
        $cands = [($fp.stdout | str trim)]
    }
    let r = (do { ^git log --format="%H %s" -n 30 $sha } | complete)
    if $r.exit_code == 0 {
        let merge_shas = (do {
            let m = (do { ^git log --merges --format="%H %s" -n 30 $sha } | complete)
            if $m.exit_code == 0 {
                parse-log $m.stdout
            } else {
                []
            }
        })
        let landed = (parse-log $r.stdout | skip 1 | where {|x|
            ($x.subject =~ '\(#\d+\)$') or ($merge_shas | any {|mrow| $mrow.sha == $x.sha})})
        $cands = ($cands | append ($landed | get sha))
    }
    for cand in ($cands | uniq) {
        let t = (trailer-run $cand)
        if $t != null {
            return {trailer_run: $t, inherited_from: $cand}
        }
    }
    null
}

# Tracker-closed arm with empty implementation matches: resolve the closing
# commit and classify ITS trailer, so the JSON never lacks closure identity.
# A seeds-close commit without its own trailer inherits from the adjacent
# landed-PR commit (fabro-cf2a); the body-grep arm never inherits.
def resolve-closing [base, id, self_id] {
    let seeds_close = (closing-seeds-commit $base $id)
    let c = (if ($seeds_close != null) { $seeds_close } else {
        # subject-truncation fallback: the seed id may sit in a commit body
        # beyond the squash subject — any body-matching commit qualifies.
        let pool = (git-log-matching $base $id []).rows
        if ($pool | is-empty) { null } else { $pool | first }
    })
    if $c == null {
        return null
    }
    let tr = (trailer-run $c.sha)
    let inh = (if $tr == null and $seeds_close != null {
        inherited-trailer $base $c.sha
    } else {
        null
    })
    let trailer_run = (if $inh != null { $inh.trailer_run } else { $tr })
    let inherited_from = (if $inh != null { $inh.inherited_from } else { null })
    {source: (if ($seeds_close != null) { "seeds-close" } else { "body-grep" }),
     sha: $c.sha,
     subject: $c.subject,
     trailer_run: $trailer_run,
     trailer_inherited_from: $inherited_from,
     closure: (classify-closure $trailer_run $self_id)}
}

def main [...ids: string, --base: string = "origin/denkhaus", --self: string] {
    if ($ids | is-empty) {
        print -e "dup-run-check: no seed id given — usage: nu .fabro/scripts/dup-run-check.nu <seed-id>... [--base <ref>] [--self <run-id>]"
        exit 2
    }
    let self_id = ($self | default null)
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

        let impl = (with-closure $implementations $self_id)
        let foreign_impl = ($impl | where {|r| $r.closure == "foreign"})

        let closing_evidence = (if $tracker_status == "closed" and ($impl | length) == 0 {
            (resolve-closing $base $id $self_id)
        } else {
            null
        })

        let tracker_closure = (if $tracker_status != "closed" {
            null
        } else if ($impl | length) > 0 {
            $impl | first | get closure
        } else if $closing_evidence != null {
            $closing_evidence.closure
        } else {
            "unknown"
        })

        let verdict = (if ($foreign_impl | length) > 0 or $tracker_closure == "foreign" or $tracker_closure == "unknown" {
            "duplicate"
        } else {
            "clean"
        })

        let closure_note = (if $verdict == "clean" and $self_id != null and (($impl | length) > 0 or $tracker_status == "closed") {
            let inh = (if $closing_evidence == null { null } else {
                $closing_evidence.trailer_inherited_from? | default null
            })
            if $inh != null {
                $"self-closure: Fabro-Run trailer inherited from ($inh)"
            } else {
                $"self-closure: Fabro-Run trailer names ($self_id)"
            }
        } else {
            null
        })

        {seed: $id,
         verdict: $verdict,
         tracker_status: $tracker_status,
         tracker_note: (if ($tracker_note | is-empty) { null } else { $tracker_note }),
         implementation_matches: $impl,
         filed_only_matches: ($filed_only | length),
         other_refs: ($other | length),
         closing_evidence: $closing_evidence,
         closure_note: $closure_note} | to json --raw | print
    }
}
