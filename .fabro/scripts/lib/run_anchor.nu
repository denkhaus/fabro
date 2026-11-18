#!/usr/bin/env nu
# The run anchor (denkhaus/work#1108): the branch/run-id/run-base triple
# existed as MIRRORED copies in closeout.nu, evidence.nu and
# check-transcript.nu ("change both together" comments holding them
# together — discipline instead of mechanism). Every consumer sources
# THIS file; none re-implements the shape. The adjudicator fast-path
# script and the stage-G reporter consume the same module.
#
# work#1445 (verification commons): the SHAPES behind this API — the
# run-id regex, the subject mark, the base derivation with its
# checkpoint-miss JOURNAL fallback (work#1427) — moved ONCE into
# scripts/lib/verification_commons.nu, where verify.nu and qualitygate.nu
# consume the same module. This file stays the develop-graph-facing
# ADAPTER: the public API is unchanged (id-from-branch / current-branch /
# run-id / subject-mark / run-base; run-base's record exactly
# {base, short, grounded}), and every def is a one-line delegation. The
# regrowth pin in scripts/verification-commons-fixtures.nu holds both
# halves (API intact, shapes not re-implemented).
#
# The off-run DEFAULT differs by consumer and stays parametrized:
# closeout/check-transcript read '' (journal paths, dedupe queries),
# evidence reads 'local' (transcript paths). run-base itself is
# default-invariant off-run: any sentinel yields the off-run HEAD arm
# (grounded false), never a run-branch arm.

use ../../../scripts/lib/verification_commons.nu

# Pure: the run id from a run branch (`fabro/run/<id>`), `off_run` when
# the branch is not a run branch. The regex anchors the END of the
# branch (`[^/]+$`): `fabro/run/a/b` is not a run branch.
export def id-from-branch [branch: string, off_run: string = '']: nothing -> string {
    verification_commons id-from-branch $branch $off_run
}

# The current git branch, trimmed ('' on a detached HEAD).
export def current-branch []: nothing -> string {
    verification_commons current-branch
}

# The run id of THIS checkout (`fabro/run/<id>` branch), `off_run` off a
# run branch.
export def run-id [off_run: string = '']: nothing -> string {
    verification_commons run-id $off_run
}

# Pure: the FIXED-STRING git grep mark for a run's checkpoint commits
# (`fabro(<id>):`). The value carries LITERAL parentheses (the source's
# `\(` is string-escaping against interpolation, not part of the value):
# with --fixed-strings the parens match literally, while a regex-shaped
# mark like `fabro.<id>:` silently matches nothing (it lacks the `)`
# before `:`), which once produced empty diff sections and misleading
# evidence.
export def subject-mark [run_id: string]: nothing -> string {
    verification_commons subject-mark $run_id
}

# Parent of THIS run's earliest checkpoint commit, identified via the
# run branch name and the fixed-string subject mark. Foreign run
# checkpoints merged into history are ignored. Arms (work#1445 healing):
#   checkpoint hit    -> its parent, grounded true;
#   checkpoint miss   -> the work#1427 JOURNAL fallback: the newest
#                        engine-written commit for this run (`Fabro-Run:`
#                        trailer or checkpoint subject) grounds the base
#                        on its parent, grounded true;
#   journal miss      -> HEAD^ (the last resort), grounded true;
#   root edge / off-run -> HEAD, grounded false.
# Every git call is complete-wrapped (degrade to the HEAD fallback, never
# a crash) — the defensive superset of the two former copies. The public
# record stays exactly {base, short, grounded}.
export def run-base []: nothing -> record<base: string, short: string, grounded: bool> {
    let r = (verification_commons run-base 'local')
    {base: $r.base, short: $r.short, grounded: $r.grounded}
}
