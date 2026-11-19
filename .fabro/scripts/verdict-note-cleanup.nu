#!/usr/bin/env nu
# The verdict-rationale note cleanup (denkhaus/work#1409): the
# adjudicator's verdict notes land untriaged in the observations feed and
# stay there forever — the triager's sweep filters category=painpoint,
# so notes never enter it, and nothing derives a terminal state for
# linkless notes. This pass dispositions the adjudicator's
# verdict-rationale notes (`verdict: ` head) as reviewed — they are audit
# records, not triage-worthy findings. Other untriaged notes (routed
# escalations, anything not matching the head filter) stay untouched.
#
# Both legs ride the CLI verbs through the work-channel seam
# (work#1475): the list leg is the global feed door (`fabro observation
# feed --category note --triage untriaged --json`, ADR-0044 — the verb
# owns auth, token refresh and server-side paging; the retired raw
# HTTP leg raced a stale token-file read), the manage leg is `fabro
# observation manage`. The binary resolves from PATH (FABRO_BIN
# overrides, the batteries' shim). Session-owned cleanup script.
#
# Verdicts (one line per record): cleanup:reviewed <run>#<seq> |
# cleanup:skip:<reason>. Exit 0 always — the routing is the caller's.

use lib/work_channel.nu [cli, observation-feed-argv]

const PAGE = 100

def main [
  --server: string = 'https://mirtuell.net'  # the production feed
  --max-records: int = 500                   # hard stop, the pass is idempotent
  --dry-run                                  # report the matches, decide nothing
] {
  mut reviewed = 0
  mut skipped = 0
  mut page = 1
  while ($reviewed + $skipped) < $max_records {
    let res = (cli 'cleanup' (observation-feed-argv $server --category 'note' --triage 'untriaged' --limit $PAGE --page $page))
    if $res.exit_code != 0 {
      print -e $"cleanup: observation feed read failed: ($res.stderr | str trim | str substring 0..200)"
      exit 1
    }
    let parsed = (try { $res.stdout | from json } catch { null })
    if not ($parsed | describe | str starts-with 'record') {
      print -e 'cleanup: observation feed stdout is not the GlobalObservationList record'
      exit 1
    }
    let observations = ($parsed | get -o observations | default [])
    if ($observations | is-empty) { break }
    for row in $observations {
      let text = ($row.text? | default '')
      let run = ($row.run_id? | default '')
      let seq = ($row.seq? | default 0)
      if not ($text | str starts-with 'verdict: ') {
        $skipped += 1
        continue
      }
      if $dry_run {
        print $"cleanup:would-review ($run)#($seq)"
      } else {
        let argv = [
          'observation' '--server' $server 'manage'
          '--run-id' $run
          '--seq' ($seq | into string)
          '--action' 'no-filing'
          '--reason' 'adjudicator verdict rationale — audit record, auto-reviewed per the feed-hygiene design (work#1409)'
        ]
        let m = (cli 'cleanup' $argv)
        if $m.exit_code == 0 {
          print $"cleanup:reviewed ($run)#($seq)"
          $reviewed += 1
        } else {
          print -e $"cleanup:mark-failed ($run)#($seq): ($m.stderr | str substring 0..160)"
        }
      }
    }
    let total = ($parsed | get -o total | default 0)
    if (($page * $PAGE) >= $total) { break }
    $page += 1
  }
  print $"cleanup: done reviewed=($reviewed) skipped-other-notes=($skipped)"
}
