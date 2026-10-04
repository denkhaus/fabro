#!/usr/bin/env nu
# Salvage sweep (fabro-f312, user directive 2026-09-30): never let the work
# of a failed run vanish silently. For every TERMINAL run in the window the
# sweep detects stranded noteworthy work - a failed or green-lie run (any
# stage failed while the run read succeeded) whose stage diffs carry real
# changes - and files ONE salvage-pointer seed citing the run, the dump
# export command and the affected seed. Deterministic script first: the
# consumer prompt keeps only the call + verdict routing.
#
# Verdicts (one line per run): salvage:none | salvage:filed <seed-id> |
# salvage:pointer-exists | sweep:skip:<reason>. Exit 0 always - the verdict
# routing is the caller's business.

const ANALYSIS = 'salvage-analysis.nu'
source $ANALYSIS

def main [
  --server: string = 'https://mirtuell.net'  # the production line
  --since: duration = 24hr                    # the sweep window
  --max-runs: int = 25                        # cap dumps per sweep
  --dry-run                                   # report verdicts, file nothing
] {
  let cutoff = ((date now) - $since | format date '%Y-%m-%dT%H:%M:%S')
  let runs = (do { ^fabro ps -a --json --server $server } | complete)
  if $runs.exit_code != 0 {
    print -e $"salvage-sweep: fabro ps failed — ($runs.stderr | str substring 0..160)"
    exit 1
  }
  let terminal = ($runs.stdout | from json | where {|r|
    ($r.status?.kind? | default 'unknown') in $TERMINAL
  } | where {|r| ($r.start_time? | default '') >= $cutoff } | first $max_runs)
  for run in $terminal {
    let dump_dir = (mktemp -d)
    let export = (do { ^fabro dump --output $dump_dir --server $server $run.run_id } | complete)
    if $export.exit_code != 0 {
      print $"sweep:skip:dump-failed ($run.run_id)"
      continue
    }
    let analysis = (analyze-dump $dump_dir)
    if not $analysis.candidate {
      print $"salvage:none ($run.run_id)"
      continue
    }
    if not $analysis.noteworthy {
      print $"salvage:none ($run.run_id) — no noteworthy work"
      continue
    }
    let known = (do { ^seeds search $run.run_id --limit 5 } | complete)
    if ($known.exit_code == 0 and ($known.stdout | str contains $run.run_id)) {
      print $"salvage:pointer-exists ($run.run_id)"
      continue
    }
    if $dry_run {
      print $"salvage:would-file ($run.run_id) — affected ($analysis.affected_seed), ($analysis.changed) changed lines, ($analysis.new_files) new files"
      continue
    }
    let goal_short = ($run.goal? | default '' | str substring 0..90)
    let today = (date now | format date '%Y-%m-%d')
    let body = ('SALVAGE POINTER (fabro-f312 sweep, ' + $today + '): terminal run ' + $run.run_id + ' — ' + $goal_short + ' — carries noteworthy stranded work (' + ($analysis.changed | into string) + ' changed lines, ' + ($analysis.new_files | into string) + ' new files); affected seed: ' + $analysis.affected_seed + '. Export: FABRO_SERVER=' + $server + ' fabro dump --output <dir> ' + $run.run_id + '. Review the stage diffs, land the work via PR citing the run, then close this pointer.')
    let title = ('salvage: run ' + $run.run_id + ' carries noteworthy stranded work')
    let filed = (do { ^seeds create --title $title --type task --priority 2 --labels salvage,revision --description $body } | complete)
    if $filed.exit_code != 0 {
      print -e $"salvage-sweep: seeds create failed for ($run.run_id) — ($filed.stderr | str substring 0..120)"
      print $"sweep:skip:file-failed ($run.run_id)"
      continue
    }
    print $"salvage:filed ($run.run_id)"
  }
  print 'salvage-sweep: done'
}
