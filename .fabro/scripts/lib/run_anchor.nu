#!/usr/bin/env nu
# The run anchor (denkhaus/work#1108): the branch/run-id/run-base triple
# existed as MIRRORED copies in closeout.nu, evidence.nu and
# check-transcript.nu ("change both together" comments holding them
# together — discipline instead of mechanism). Every consumer sources
# THIS file; none re-implements the shape. The adjudicator fast-path
# script and the stage-G reporter consume the same module.
#
# The off-run DEFAULT differs by consumer and stays parametrized:
# closeout/check-transcript read '' (journal paths, dedupe queries),
# evidence reads 'local' (transcript paths). run-base itself is
# default-invariant off-run: any sentinel yields the never-matching
# grep mark and the HEAD fallback.

# Pure: the run id from a run branch (`fabro/run/<id>`), `off_run` when
# the branch is not a run branch. The regex anchors the END of the
# branch (`[^/]+$`): `fabro/run/a/b` is not a run branch.
export def id-from-branch [branch: string, off_run: string = '']: nothing -> string {
    let id = ($branch | parse --regex 'fabro/run/(?P<id>[^/]+)$' | get -o id.0 | default '')
    if ($id | is-empty) { $off_run } else { $id }
}

# The current git branch, trimmed ('' on a detached HEAD).
export def current-branch []: nothing -> string {
    git branch --show-current | str trim
}

# The run id of THIS checkout (`fabro/run/<id>` branch), `off_run` off a
# run branch.
export def run-id [off_run: string = '']: nothing -> string {
    id-from-branch (current-branch) $off_run
}

# Pure: the FIXED-STRING git grep mark for a run's checkpoint commits
# (`fabro(<id>):`). The value carries LITERAL parentheses (the source's
# `\(` is string-escaping against interpolation, not part of the value):
# with --fixed-strings the parens match literally, while a regex-shaped
# mark like `fabro.<id>:` silently matches nothing (it lacks the `)`
# before `:`), which once produced empty diff sections and misleading
# evidence.
export def subject-mark [run_id: string]: nothing -> string {
    $"fabro\(($run_id)\):"
}

# Parent of THIS run's oldest checkpoint commit, identified via the run
# branch name and the fixed-string subject mark. Foreign run checkpoints
# merged into history are ignored. Fallback: HEAD (`grounded: false`).
# Every git call is complete-wrapped (degrade to the HEAD fallback, never
# a crash) — the defensive superset of the two former copies.
export def run-base []: nothing -> record<base: string, short: string, grounded: bool> {
    let run_id = (run-id 'local')
    let mark = (subject-mark $run_id)
    let checkpoints = (do { git log --format=%H --fixed-strings --grep $mark } | complete | get stdout | lines | compact)
    if ($checkpoints | is-empty) {
        {base: "HEAD", short: (do { git rev-parse --short HEAD } | complete | get stdout | str trim), grounded: false}
    } else {
        let base = (do { git rev-parse $"($checkpoints | last)^" } | complete | get stdout | str trim)
        {base: $base, short: (do { git rev-parse --short $base } | complete | get stdout | str trim), grounded: true}
    }
}
