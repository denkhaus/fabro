#!/usr/bin/env nu
# Landed-but-open sweep (denkhaus/work#1553, the acceptance chain's
# pool-hygiene part): the correlated-failure evidence included items
# whose PRs MERGED while the items stayed open (#1533/#1469/#1535) —
# nobody looked, because every layer consumed the same prose scope.
# This sweep flags them mechanically: a run the merger marked
# closable=enabled is INTEGRATED; its claimed item's acceptance fold
# then reads `closable_landed` until the closing pass closes it — an
# entry that stays there is landed-but-open, exactly the class the
# sprint ledgers once miscounted as closed.
#
# Deterministic script first (the line-watch procedure keeps only the
# call + verdict routing): the read rides the work-channel seam
# (lib/work_channel.nu, work#1475) through the `fabro run queue` verb
# — the verb owns auth and token refresh; no raw HTTP, no forge
# client (ADR-0019).
#
# Verdicts (one line per flagged item):
#   landed-open:<item key> integrated by <run>, still open — close or
#   account it
# Exit 0 when the pool is clean, 1 when anything is flagged.

use lib/work_channel.nu [read-queue-hard]

# Pure: one queue entry is landed-but-open when the merger settled it
# (closable enabled) and the item's acceptance fold still reads
# closable_landed — integrated, not closed, and going nowhere.
export def is-landed-open [entry]: nothing -> bool {
    ($entry.acceptance? | default '') == 'closable_landed'
}

# Pure: the flag line for one landed-but-open entry.
export def flag-line [entry]: nothing -> string {
    let run = ($entry.run?.id? | default ($entry.run? | default '?'))
    let item = ($entry.work_item? | default '?')
    $"landed-open:($item) integrated by ($run), still open — close or account it"
}

def main [
  --workflow: string = 'develop'        # whose closable queue to sweep
  --limit: int = 200                    # hard stop, the pass is idempotent
] {
    let entries = (read-queue-hard 'landed-open-sweep' 'closable' $workflow 'enabled')
    let flagged = ($entries | where {|entry| (is-landed-open $entry)} | first $limit)
    if ($flagged | is-empty) {
        print $"landed-open-sweep: clean — no integrated-but-open items on ($workflow)"
        exit 0
    }
    for entry in $flagged {
        print (flag-line $entry)
    }
    exit 1
}
