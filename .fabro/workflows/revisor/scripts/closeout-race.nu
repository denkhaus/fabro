#!/usr/bin/env nu
# Closeout-race listing (fabro-74d4): the race where the TARGET run's
# own closeout files seeds WHILE the revisor reviews it. The revisor's
# review snapshot (the fabro_ask answer) is generated at analyze start;
# the target run's closeout PR can merge — filing follow-up seeds that
# cite the run — inside the window between that snapshot and the
# revisor's filing stage. An analyst deduping only against its
# start-of-analysis tracker read re-derives already-filed work, and the
# bookkeeper then has to merge it away at zero filing credit.
#
# This script is the deterministic surface that closes the window: it
# lists, from the LIVE tracker, every seed whose title or body cites the
# target run id — the closeout's own filings plus anything else grounded
# in that run. The analyst runs it before distilling; the bookkeeper
# re-runs it before the first `seeds create` (filing time). Content
# matching (does a listed seed COVER a recommendation?) stays a judgment
# call in the prompts — this script only makes the candidate set
# impossible to miss.
#
# Fail-open: on any internal error (seeds CLI missing, unreadable
# output, bad --tracker file) the script prints an `outcome: degraded`
# object and exits 0 — a guard's own failure must never block a pass;
# the prompts treat degraded as "re-run `seeds list` manually and
# dedupe by eye".
#
# Usage:
#   nu closeout-race.nu <run-id> [--since <ISO8601>] [--tracker <jsonl>]
#     <run-id>   the TARGET run id (revisor_target_run_id), never the
#                consuming revisor run's own id
#     --since    optional watermark: only seeds with updatedAt strictly
#                after this ISO8601 instant (the review-snapshot moment)
#                are listed — the narrow race-window view
#     --tracker  fixture override: a JSONL file of issue objects (the
#                `seeds show` per-issue shape, one per line) instead of
#                the live `seeds list` call — the fixtures battery pins
#                behavior against it; production never passes it
#
# Output: one JSON object
#   {"outcome":"ok","run":"<id>","since":<since|null>,"count":N,
#    "matching":[{"id","title","status","createdAt","updatedAt"}]}
#   {"outcome":"degraded","error":"..."}   (fail-open, exit 0)
# Exit 0 unless the invocation itself is wrong (then 2).

# One optional field from a possibly-heterogeneous record.
def field [rec, name: string] {
    try { $rec | get $name } catch { null }
}

# Match: the run id appears in the seed's title or description/body.
def cites-run [seed, run_id: string] {
    let title = (field $seed 'title')
    let desc = (field $seed 'description')
    ((($title | default '') | str contains $run_id) or ((($desc | default '') | str contains $run_id)))
}

def main [run_id: string, --since: string, --tracker: string] {
    if ($run_id | is-empty) {
        print -e 'closeout-race: a target run id is required'
        exit 2
    }

    # Seed pool: fixture JSONL when overridden, else the live tracker
    # (open AND closed — a closed closeout seed still covers).
    let pool = if ($tracker | is-not-empty) {
        if not ($tracker | path exists) {
            print ({outcome: 'degraded', run: $run_id, error: $"tracker fixture not found: ($tracker)" } | to json -r)
            return
        }
        let text = (try { open --raw $tracker } catch { null })
        if $text == null {
            print ({outcome: 'degraded', run: $run_id, error: 'tracker fixture unreadable' } | to json -r)
            return
        }
        $text
        | lines
        | compact
        | each {|l| try { $l | from json } catch { null }}
        | compact
    } else {
        let r = (do { ^seeds list --format json --all --limit 500 } | complete)
        if $r.exit_code != 0 {
            print ({outcome: 'degraded', run: $run_id, error: $"seeds list failed: ($r.stderr)" } | to json -r)
            return
        }
        let parsed = (try { $r.stdout | from json } catch { null })
        let issues = (try { $parsed | get issues } catch { null })
        if ($issues == null) {
            print ({outcome: 'degraded', run: $run_id, error: 'seeds list output carried no issues array' } | to json -r)
            return
        }
        $issues
    }

    let matching = ($pool
        | where {|s| cites-run $s $run_id}
        | each {|s|
            {id: (field $s 'id'),
             title: (field $s 'title'),
             status: (field $s 'status'),
             createdAt: (field $s 'createdAt'),
             updatedAt: (field $s 'updatedAt')}
        })

    let windowed = if ($since | is-empty) {
        $matching
    } else {
        # ISO8601 Zulu instants compare correctly as strings.
        $matching | where {|m| ($m.updatedAt | default '' | str trim) > $since }
    }

    print ({outcome: 'ok', run: $run_id, since: ($since | default null), count: ($windowed | length), matching: $windowed } | to json -r)
}
