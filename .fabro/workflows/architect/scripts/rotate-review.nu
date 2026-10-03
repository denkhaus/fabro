#!/usr/bin/env nu
# Deterministic same-date review rotation (fabro-491b): runs BEFORE the
# analyze stage writes its plain-date review file. When
# `<reviews-dir>/<today YYYY-MM-DD>.md` already exists (an earlier
# same-date pass — forced or manual, the cooldown marker gates only the
# cron path), the existing file is preserved verbatim as a timestamped
# sibling `<date>-<HHmm>.md` with HHmm taken from the existing file's
# mtime (fallback: current time). If the timestamped name is already
# taken, `-2`, `-3`, ... are appended. The plain-date name therefore
# always carries the LATEST pass and no earlier prose is lost.
#
# Fail-open (hard rule): rotation must never block an architecture pass.
# A missing plain-date file is the healthy common case — plain no-op,
# exit 0, no side effects. Any internal error is caught, journaled to
# stderr, and still exits 0.
#
# Usage (workflow script node; also drivable standalone):
#   nu rotate-review.nu [--reviews-dir <path>]   # default .fabro/architecture/reviews

def main [
    --reviews-dir: path = '.fabro/architecture/reviews'  # reviews tree to rotate
] {
    let today = (date now | format date '%Y-%m-%d')
    let plain = ($reviews_dir | path join $"($today).md")

    if not ($plain | path exists) {
        # No earlier same-date pass: nothing to rotate.
        return
    }

    try {
        # Timestamp from the existing file's mtime; fall back to now.
        let mtime = (ls --long $plain | get 0.modified)
        let hhmm = ($mtime | format date '%H%M')

        # Disambiguate `<date>-<HHmm>.md`, then `-2`, `-3`, ...
        let base = ($reviews_dir | path join $"($today)-($hhmm).md")
        mut target = $base
        mut n = 1
        while ($target | path exists) {
            $n = $n + 1
            $target = ($reviews_dir | path join $"($today)-($hhmm)-($n).md")
        }

        cp $plain $target
        print $"rotate-review: preserved prior pass ($plain | path basename) -> ($target | path basename)"
    } catch {|err|
        # Fail-open: an mtime/stat/copy hiccup must never block the pass.
        print --stderr $"rotate-review: rotation failed (fail-open): ($err.msg)"
    }
}
