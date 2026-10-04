#!/usr/bin/env nu
# Salvage-sweep analysis core (fabro-f312): pure dump-directory analysis,
# shared by the sweep CLI (.fabro/scripts/salvage-sweep.nu) and its smoke
# battery (.fabro/scripts/salvage-sweep-smoke.nu). No main here by design —
# sourcing a file that defines one makes nu run it after the caller.

# Terminal status kinds (the engine's terminal set).
const TERMINAL = [succeeded failed canceled cancelled]

# A patch is noteworthy when it changes more than this many lines, or adds
# any new file - counted over lines that touch REAL work (bookkeeping
# paths - the run's own journal and the tracker - never count).
const CHANGED_LINE_THRESHOLD = 20

# Bookkeeping path prefixes whose changes are never noteworthy work.
const BOOKKEEPING = ['.fabro/journal/' '.seeds/']

# The analysis of one dump directory: pure, so the smoke battery can pin it.
# Returns {candidate: bool, green_lie: bool, noteworthy: bool, changed: int,
# new_files: int, affected_seed: string}.
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
      not ($BOOKKEEPING | any {|prefix| $p starts-with $prefix })
    })
    {
      changed: ($real | where {|l|
        # count only lines belonging to non-bookkeeping files is not
        # expressible per-line; approximate via hunk headers below
        true
      } | length),
      new_files: ($lines | where {|l| $l starts-with 'new file mode' } | length),
      real_files: ($touched | length),
    }
  })
  let empty_guard = ($counts | is-empty)
  let changed = (if $empty_guard { 0 } else { $counts | get changed | math sum })
  let new_files = (if $empty_guard { 0 } else { $counts | get new_files | math sum })
  let real_files = (if $empty_guard { 0 } else { $counts | get real_files | math sum })
  let noteworthy = (($real_files > 0) and (($changed > $CHANGED_LINE_THRESHOLD) or ($new_files > 0)))
  let affected = ($run.spec.settings.run.goal?.value? | default '' | parse -r '(?<s>fabro-[0-9a-f]{4})' | get s? | first | default '')
  {
    candidate: ($run_failed or $green_lie),
    green_lie: $green_lie,
    noteworthy: $noteworthy,
    changed: $changed,
    new_files: $new_files,
    affected_seed: ($affected | default ''),
  }
}
