#!/usr/bin/env nu
# close-claim-check (fabro-2a3b): every `close <seed>` claim in a commit
# subject must be true in the tracker. A subject can assert a close its
# diff never performed (fabro-a9bd: open ~9h with the work merged), which
# skews the ready queue and invites a re-claim of landed work.
#
# Two faces, one rule:
#   nu .fabro/scripts/close-claim-check.nu            # live: last 25 commits
#   nu .fabro/scripts/close-claim-check.nu --last 100 # deeper history
# The pure core (close-claims + close-claim-findings) is exported for the
# smoke battery; `source` here is safe because main only runs when invoked.

# Seed ids a subject claims to close: after the word close/closes/closed/
# closing, collect fabro-<id> tokens while only ids and list separators
# (`,`, `+`, `and`) follow. Anything else ends the list — `reconcile
# fabro-x`, `file fabro-x`, `residual: fabro-x` are NOT close claims.
export def close-claims [subject: string] {
    let words = ($subject | split row ' ')
    mut claims = []
    mut collecting = false
    for word in $words {
        let id = ($word | parse --regex '(?P<id>fabro-[a-z0-9]+)' | get -o id.0?)
        if $collecting {
            if ($id != null) {
                $claims = ($claims | append $id)
            } else if (not ($word in [',' '+' 'and' '+,' ',+' ';'])) {
                $collecting = false
            }
        }
        if (not $collecting) and ($word in ['close' 'closes' 'closed' 'closing' 'Close' 'Closes']) {
            $collecting = true
        }
    }
    $claims | uniq
}

# Findings for (subjects, tracker records): a claimed id whose record is
# missing entirely is a finding too — an id nobody can resolve cannot be
# closed. Records: [{id status}]; subjects: [{sha subject}].
export def close-claim-findings [subjects, records] {
    let statuses = ($records | select id status)
    $subjects
    | each {|commit|
        close-claims $commit.subject
        | each {|id|
            let status = ($statuses | where id == $id | get -o status.0?)
            if $status != 'closed' {
                {commit: $commit.sha, seed: $id, recorded_status: ($status | default 'missing')}
            }
        }
    }
    | flatten
    | where {|row| $row != null}
}

def main [--last: int = 25] {
    let commits = (git log --format='%h %s' -n $last
        | lines
        | parse '{sha} {subject}')
    let claims = ($commits
        | each {|commit| {sha: $commit.sha, ids: (close-claims $commit.subject)}}
        | where {|row| ($row.ids | length) > 0})
    if ($claims | is-empty) {
        print $"close-claim-check: no close claims in the last ($last) commits"
        return
    }
    print ($claims | each {|row| $"($row.sha): ($row.ids | str join ', ')"})
    # One seeds show per claimed id; the tracker is the only truth here.
    let records = ($claims
        | get ids
        | flatten
        | uniq
        | each {|id|
            let show = (do { ^seeds show $id --json } | complete)
            if $show.exit_code == 0 {
                $show.stdout | from json | get issue | select id status
            } else {
                {id: $id, status: 'missing'}
            }
        })
    let findings = (close-claim-findings $commits $records)
    if ($findings | is-empty) {
        print 'close-claim-check: green — every close claim is true in the tracker'
    } else {
        print 'close-claim-check: RED — close claims their diff never performed:'
        $findings | each {|row| print $"  ($row.commit) claims close ($row.seed) (recorded: ($row.recorded_status))"}
        exit 1
    }
}
