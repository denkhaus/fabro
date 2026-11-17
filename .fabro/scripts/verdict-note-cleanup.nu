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
# The list leg rides the global feed door over HTTP (the CLI's
# observation list is run-scoped only); the manage leg rides the TREE
# build's fabro observation manage (the installed fork.36 binary predates
# the subcommand). Session-owned cleanup script.
#
# Verdicts (one line per record): cleanup:reviewed <run>#<seq> |
# cleanup:skip:<reason>. Exit 0 always — the routing is the caller's.

const PAGE = 100
const FABRO = '/home/denkhaus/dev/fabro/target/debug/fabro'

def main [
  --server: string = 'https://mirtuell.net'  # the production feed
  --max-records: int = 500                   # hard stop, the pass is idempotent
  --dry-run                                  # report the matches, decide nothing
] {
  let token = (open ~/.fabro/auth.json | get servers | get $server | get -o access_token | default '')
  if ($token | is-empty) {
    print -e $"cleanup: no access token for ($server) — run one fabro CLI call first to refresh"
    exit 1
  }
  let headers = {Authorization: $"Bearer ($token)"}
  mut reviewed = 0
  mut skipped = 0
  mut page = 1
  while ($reviewed + $skipped) < $max_records {
    let qs = $"category=note&triage=untriaged&limit=($PAGE)&page=($page)"
    let rows = (http get --headers $headers $"($server)/api/v1/observations?($qs)")
    let observations = ($rows | get -o observations | default [])
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
        let argv2 = ['observation' 'manage', '--run-id' $run, '--seq' ($seq | into string), '--action' 'no-filing', '--reason' 'adjudicator verdict rationale — audit record, auto-reviewed per the feed-hygiene design (work#1409)']
        let m = (do { ^$FABRO ...$argv2 } | complete)
        if $m.exit_code == 0 {
          print $"cleanup:reviewed ($run)#($seq)"
          $reviewed += 1
        } else {
          print -e $"cleanup:mark-failed ($run)#($seq): ($m.stderr | str substring 0..160)"
        }
      }
    }
    let total = ($rows | get -o total | default 0)
    if (($page * $PAGE) >= $total) { break }
    $page += 1
  }
  print $"cleanup: done reviewed=($reviewed) skipped-other-notes=($skipped)"
}
