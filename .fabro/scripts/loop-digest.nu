#!/usr/bin/env nu
# Loop-wide journal painpoint digest (deterministic, architect survey input).
#
# The architect reviews the LOOP AS A SYSTEM: the interplay of all
# workflows, not any single run. This script aggregates every workflow's
# stage journals into one compact JSON array: per run - its workflow
# (derived from the node signature) and its painpoint texts (capped).
# The survey summarizes this into the `arch_loop_digest` context key;
# the analyze stage treats it as the PRIMARY evidence of system-level
# friction. Single runs are instances, never the subject.
#
# Workflow signatures (stable node vocabulary):
#   planner          -> develop
#   select           -> revisor
#   analyze          -> architect
#   anything else    -> other (conductor legs classify here by design;
#                       the conductor's painpoints surface via children)
#
# Usage: nu .fabro/scripts/loop-digest.nu [--days 7]

def main [--days: int = 7] {
    let cutoff = ((date now) - ($days * 1day))
    ls .fabro/journal/*.jsonl
    | where {|f| $f.modified > $cutoff }
    | sort-by modified --reverse
    | each {|f|
        let recs = (open --raw $f.name | lines | compact
            | each {|l| try { $l | from json } catch { null } }
            | where {|r| $r != null })
        let nodes = ($recs | get node | uniq)
        let workflow = if ($nodes | any {|n| $n == 'planner'}) {
            'develop'
        } else if ($nodes | any {|n| $n == 'select'}) {
            'revisor'
        } else if ($nodes | any {|n| $n == 'analyze'}) {
            'architect'
        } else {
            'other'
        }
        let pains = ($recs
            | each {|r| $r.data?.painpoints? }
            | flatten
            | where {|p| $p != null }
            | each {|p|
                if ($p | describe) == 'string' {
                    $p
                } else {
                    $p.text? | default ($p | to json)
                }
            }
            | each {|t| $t | str substring 0..200 })
        {
            run: ($f.name | path basename | str replace '.jsonl' ''),
            workflow: $workflow,
            status: ($recs | last | default {} | get -o status | default ''),
            painpoints: $pains
        }
    }
    | to json
}
