#!/usr/bin/env nu
# Salvage-sweep analysis core (fabro-f312, extended 2026-10-04): pure
# dump-directory analysis, shared by the sweep CLI
# (.fabro/scripts/salvage-sweep.nu) and its smoke battery
# (.fabro/scripts/salvage-sweep-smoke.nu). No main here by design —
# sourcing a file that defines one makes nu run it after the caller.

# Shared item-id predicate (lib/seed_id.nu): the goal's affected item uses
# the one accept-set — short and family shapes. (`use`, not `source`: this
# file is itself sourced by the sweep CLI and its smoke battery, and nu
# allows no nested `source`.)
use lib/seed_id.nu seed-ids-in

# The terminal status kinds live in lib/terminal_status.nu (work#1446,
# the one predicate); the sweep classifies through it — this file's
# retired inline const (succeeded/failed/canceled/cancelled, no
# human_needed, no dead) is gone.

# A patch is noteworthy when it changes more than this many lines, or adds
# any new file - counted over lines that touch REAL work (the run's own
# journal never counts).
const CHANGED_LINE_THRESHOLD = 20

# Bookkeeping path prefixes whose changes are not work on their own.
#
# `.fabro/journal/` never counts: the stage-journal hook writes ONE NEW file
# per run, so counting it would make every terminal run noteworthy and turn
# the sweep into noise.
const BOOKKEEPING = ['.fabro/journal/']

# The work-item WRITE tools (the run-tool registry's FABRO_WORK_*_TOOL_NAME
# constants): a terminal run that CALLED one of these strands its effect in
# the sandbox the platform deletes — a filed item, an edited body, a
# closure, a comment thread that never reached a landed PR. The read/claim
# tools (`fabro_work_next`/`fabro_work_show`/`fabro_work_list`
# /`fabro_work_release`/`fabro_work_radius`) are excluded by construction —
# every develop run calls them, so counting them would turn the sweep into
# noise.
#
# Detector vocabulary note (work#1477): the retired tracker-hunk counter
# (`added-seed-records`/`TRACKER`) watched a store that left the tree at the
# tracker cutover (denkhaus/work#988/#990, 2026-10-26) and could never fire
# again; the work-item write calls are the stranding class that exists
# post-cutover.
const WORK_TOOLS = ['fabro_work_create' 'fabro_work_close' 'fabro_work_update' 'fabro_work_comments']

# The machine-written record surface of one dump (run.json, the stage
# completion records, the stream export): the only files where a tool CALL
# sits on the record. Prompts, responses, output logs and diffs are model
# prose and code surfaces — both quote tool names without calling them
# (the workflow prompts document every tool; a diff editing this detector
# carries its literals) — so they never count.
def work-tool-records [dir: path]: nothing -> string {
  glob ($dir | path join '**' '*.json')
  | append ($dir | path join 'events.jsonl')
  | where {|f| ($f | path type) == 'file' }
  | each {|f| open --raw $f }
  | str join (char nl)
}

# Distinct work-item WRITE-tool names the records show. Each distinct name
# is one stranded-effect class the run exercised; a single call surfaces in
# both the call and the result progress events, so a raw occurrence count
# would double — the distinct-name count stays honest without parsing
# record shapes.
def work-tool-filings [records: string]: nothing -> int {
  $WORK_TOOLS | where {|t| $records | str contains $t } | uniq | length
}

# The analysis of one dump directory: pure, so the smoke battery can pin it.
# Returns {candidate: bool, green_lie: bool, noteworthy: bool, changed: int,
# new_files: int, work_filings: int, affected_item: string}.
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
      changed: ($real | length),
      new_files: ($lines | where {|l| $l starts-with 'new file mode' } | length),
      real_files: ($touched | length),
    }
  })
  let empty_guard = ($counts | is-empty)
  let changed = (if $empty_guard { 0 } else { $counts | get changed | math sum })
  let new_files = (if $empty_guard { 0 } else { $counts | get new_files | math sum })
  let real_files = (if $empty_guard { 0 } else { $counts | get real_files | math sum })
  let work_filings = (work-tool-filings (work-tool-records $dir))
  # Work-item write calls are stranded work in their own right: the run
  # filed/edited/closed items from a sandbox that is gone.
  let noteworthy = (((($real_files > 0) and (($changed > $CHANGED_LINE_THRESHOLD) or ($new_files > 0))) or ($work_filings > 0)))
  let affected = (seed-ids-in ($run.spec.settings.run.goal?.value? | default '') | first | default '')
  {
    candidate: ($run_failed or $green_lie),
    green_lie: $green_lie,
    noteworthy: $noteworthy,
    changed: $changed,
    new_files: $new_files,
    work_filings: $work_filings,
    affected_item: ($affected | default ''),
  }
}
