#!/usr/bin/env nu
# Salvage-sweep analysis core (fabro-f312, extended 2026-10-04): pure
# dump-directory analysis, shared by the sweep CLI
# (.fabro/scripts/salvage-sweep.nu) and its smoke battery
# (.fabro/scripts/salvage-sweep-smoke.nu). No main here by design —
# sourcing a file that defines one makes nu run it after the caller.

# Terminal status kinds (the engine's terminal set).
const TERMINAL = [succeeded failed canceled cancelled]

# A patch is noteworthy when it changes more than this many lines, or adds
# any new file - counted over lines that touch REAL work (the run's own
# journal and a tracker REWRITE never count).
const CHANGED_LINE_THRESHOLD = 20

# Bookkeeping path prefixes whose changes are not work on their own.
#
# `.fabro/journal/` never counts: the stage-journal hook writes ONE NEW file
# per run, so counting it would make every terminal run noteworthy and turn
# the sweep into noise.
#
# `.seeds/` counts ONLY as added records (see `added-seed-records`): a run
# that filed seeds and then died strands them in its unpublished sandbox
# (fabro-f312 closeout, 2026-10-04), while a claim or a status rewrite is
# churn that every pass produces.
const BOOKKEEPING = ['.fabro/journal/']
const TRACKER = ['.seeds/']

# Seed ids that a tracker hunk ADDS: an id on an added line that no removed
# line carries. A rewrite (claim, status change, close) keeps its id on both
# sides and never counts.
def added-seed-records [lines: list<string>]: nothing -> int {
  let removed = ($lines | where {|l| $l starts-with '-' } | str join "\n" | parse -r 'fabro-(?<id>[0-9a-f]{4})' | get id? | default [])
  let added = ($lines | where {|l| $l starts-with '+' } | str join "\n" | parse -r 'fabro-(?<id>[0-9a-f]{4})' | get id? | default [])
  let fresh = ($added | where {|id| not ($removed | any {|old| $old == $id }) })
  ($fresh | uniq | length)
}

# The diff lines of the tracker files in one patch: every line after the
# `+++ b/.seeds/...` header up to the next file header.
def tracker-hunk-lines [lines: list<string>]: nothing -> list<string> {
  mut current = ''
  mut kept = []
  for line in $lines {
    if ($line | str starts-with '+++ b/') {
      $current = ($line | str replace -r '^\+\+\+ b/' '' | str trim)
    } else if ($current != '' and ($TRACKER | any {|prefix| $current starts-with $prefix })) {
      $kept = ($kept | append $line)
    }
  }
  $kept
}

# The analysis of one dump directory: pure, so the smoke battery can pin it.
# Returns {candidate: bool, green_lie: bool, noteworthy: bool, changed: int,
# new_files: int, added_seeds: int, affected_seed: string}.
def analyze-dump [dir: path]: nothing -> record {
  let run = (open ($dir | path join 'run.json'))
  let stages = (glob ($dir | path join 'stages' '*' 'status.json'))
  let stage_outcomes = ($stages | each { open $in | get outcome })
  let run_failed = ($run.status.kind in [failed])
  let green_lie = ($run.status.kind in [succeeded] and ($stage_outcomes | any {|o| $o == 'failed' }))
  let diffs = (glob ($dir | path join 'stages' '*' 'diff.patch'))
  let counts = ($diffs | each {|patch|
    let lines = (open --raw $patch | lines)
    let real = ($lines | where {|l|
      ($l starts-with '+' or $l starts-with '-') and not ($l starts-with '+++' or $l starts-with '---')
    })
    let touched = ($lines | where {|l| $l starts-with '+++ b/' } | each {|l| $l | str replace -r '^\+\+\+ b/' '' | str trim } | where {|p|
      not (($BOOKKEEPING | append $TRACKER) | any {|prefix| $p starts-with $prefix })
    })
    {
      changed: ($real | length),
      new_files: ($lines | where {|l| $l starts-with 'new file mode' } | length),
      real_files: ($touched | length),
      added_seeds: (added-seed-records (tracker-hunk-lines $lines)),
    }
  })
  let empty_guard = ($counts | is-empty)
  let changed = (if $empty_guard { 0 } else { $counts | get changed | math sum })
  let new_files = (if $empty_guard { 0 } else { $counts | get new_files | math sum })
  let real_files = (if $empty_guard { 0 } else { $counts | get real_files | math sum })
  let added_seeds = (if $empty_guard { 0 } else { $counts | get added_seeds | math sum })
  # Added tracker records are stranded work in their own right: the run filed
  # seeds and never published them.
  let noteworthy = (((($real_files > 0) and (($changed > $CHANGED_LINE_THRESHOLD) or ($new_files > 0))) or ($added_seeds > 0)))
  let affected = ($run.spec.settings.run.goal?.value? | default '' | parse -r '(?<s>fabro-[0-9a-f]{4})' | get s? | first | default '')
  {
    candidate: ($run_failed or $green_lie),
    green_lie: $green_lie,
    noteworthy: $noteworthy,
    changed: $changed,
    new_files: $new_files,
    added_seeds: $added_seeds,
    affected_seed: ($affected | default ''),
  }
}
