#!/usr/bin/env nu
# Verification commons (denkhaus/work#1445, architecture candidate C1):
# the diff-scope and nextest-classification shapes that verify.nu,
# qualitygate.nu and run_anchor.nu each carried as mirrored copies — ONE
# diff-scope seam. Every consumer sources (`use`/`source`) THIS module;
# no consumer re-implements the shapes (the regrowth pin in
# scripts/verification-commons-fixtures.nu holds that).
#
# Owned shapes, exactly once:
#   - the run-id parse (`fabro/run/<id>` branch pattern) and the
#     fixed-string checkpoint subject mark (the work#1108 anchor triple;
#     .fabro/scripts/lib/run_anchor.nu stays the develop-graph adapter
#     and delegates here);
#   - run-base: checkpoint hit + the checkpoint-miss JOURNAL fallback
#     (work#1427) + the HEAD^ last resort;
#   - the crate-extraction regex (four former sites: verify.nu x3,
#     qualitygate.nu x1);
#   - test-file / inline-cfg(test) classification (fabro-2f70);
#   - the web-surface predicate (work#1304);
#   - the vendor-surface predicate (denkhaus/work#1444);
#   - the clap-surface predicate and the help-snapshot scope rule
#     (denkhaus/work#1429): args.rs/main.rs in a crate's diff put that
#     crate's cmd::*::help family in its nextest scope;
#   - the nextest Summary parser with the work#1424/#1432 verdict
#     distinction: "0 tests matched the filter" is a legit gate FAIL,
#     "output unparseable" is a tooling error — never a gate verdict.
#
# Regex hygiene (run 01M2GVW7GGGB, lint-nu's interpolated-regex scan):
# every regex here is a PLAIN string; regex groups never travel inside
# $'...' / $"..." interpolation.

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

def short-of [rev: string]: nothing -> string {
    do { git rev-parse --short $rev } | complete | get stdout | str trim
}

# The newest commit identifiable as THIS run's engine-written journal
# commit, '' when none. Detection (work#1427): engine-written commits —
# journal, auto-commit, checkpoint — carry the `Fabro-Run: <id>` trailer
# (see any merge commit by fabro-the-factory in the log); checkpoint
# commits additionally carry the `fabro(<id>):` subject. ONE git log,
# both marks OR'd (git's default for multiple --grep), --fixed-strings
# so the parentheses stay literal. Bounded to the recent window like the
# checkpoint scan: recovery work sits at the branch tip.
def newest-journal-sha [run_id: string]: nothing -> string {
    let trailer = ('Fabro-Run: ' + $run_id)
    let res = (do { git log --format=%H -n 40 --fixed-strings --grep (subject-mark $run_id) --grep $trailer } | complete)
    if $res.exit_code != 0 { return '' }
    $res.stdout | lines | compact | get -o 0 | default ''
}

# The run base: WHAT the diff scopes against, and HOW it was anchored.
# Arms, in decision order:
#   off-run (not a run branch)           -> the consumer's draft scope:
#       default: HEAD (uncommitted changes), how 'off-run', grounded
#       false; --origin-branch: origin/<branch> tip when it exists
#       (verify.nu's arm), else HEAD, how 'origin'.
#   checkpoint hit                       -> parent of the picked
#       checkpoint commit (subject `fabro(<id>):`), how 'checkpoint'.
#       --checkpoint-pick: 'earliest' = the run's FIRST checkpoint
#       (whole-run scope — the qualitygate/run_anchor contract),
#       'latest' = the newest (stage scope — verify.nu's contract).
#       Both consumers keep their historical pick.
#   checkpoint MISS (work#1427 fallback) -> parent of the NEWEST
#       engine-written journal commit for this run (trailer OR subject —
#       see newest-journal-sha), how 'journal', grounded true.
#   journal miss, parent exists          -> HEAD^, how 'head-parent'
#       (LAST resort), grounded true.
#   root edge (no parent at all)         -> HEAD, how 'head', grounded
#       false.
#
# WHY the fallback grounds on the newest journal commit and NOT on an
# unconditional HEAD^ (the ac624e096a interim): the demand (work#1427)
# is "the last journal commit's diff" as the scope source. base =
# journal^ yields exactly that diff on a clean tree (HEAD == the journal
# commit) and only ever WIDENS when later commits exist — verify more,
# never less. Unconditional HEAD^ under-scopes: when the newest commit
# is not itself the journal commit (a shell fixup commit, a merge), it
# scopes to that one commit and silently skips the committed-but-
# unverified work the fallback exists to catch — the empty-or-thin-scope
# class that burns a gate cycle on "nothing to verify". HEAD^ therefore
# remains ONLY as the last resort when no commit identifies the run at
# all, and HEAD only for the root-commit edge. Merge-commit edge: the
# trailer grep can hit a merge; rev-parse takes its FIRST parent —
# over-inclusive (covers the whole merged branch), still never under.
export def run-base [
    off_run: string = ''
    --origin-branch
    --checkpoint-pick: string = 'earliest'
]: nothing -> record<base: string, short: string, grounded: bool, how: string> {
    let branch = (current-branch)
    let run_id = (id-from-branch $branch $off_run)
    if $run_id == $off_run {
        if $origin_branch {
            let origin = $"origin/($branch)"
            let has_origin = ((do { git rev-parse --verify --quiet $origin } | complete | get exit_code) == 0)
            if $has_origin {
                let base = (do { git rev-parse $origin } | complete | get stdout | str trim)
                {base: $base, short: (short-of $base), grounded: true, how: 'origin'}
            } else {
                {base: 'HEAD', short: (short-of 'HEAD'), grounded: false, how: 'origin-missing'}
            }
        } else {
            {base: 'HEAD', short: (short-of 'HEAD'), grounded: false, how: 'off-run'}
        }
    } else {
        let checkpoints = (do { git log --format=%H --fixed-strings --grep (subject-mark $run_id) } | complete | get stdout | lines | compact)
        if ($checkpoints | is-not-empty) {
            let sha = (if $checkpoint_pick == 'latest' { $checkpoints | first } else { $checkpoints | last })
            let base = (do { git rev-parse $"($sha)^" } | complete | get stdout | str trim)
            {base: $base, short: (short-of $base), grounded: true, how: 'checkpoint'}
        } else {
            let journal = (newest-journal-sha $run_id)
            if ($journal | is-not-empty) {
                let base = (do { git rev-parse $"($journal)^" } | complete | get stdout | str trim)
                {base: $base, short: (short-of $base), grounded: true, how: 'journal'}
            } else {
                let has_parent = ((do { git rev-parse --verify --quiet 'HEAD^' } | complete | get exit_code) == 0)
                if $has_parent {
                    let base = (do { git rev-parse 'HEAD^' } | complete | get stdout | str trim)
                    {base: $base, short: (short-of $base), grounded: true, how: 'head-parent'}
                } else {
                    {base: 'HEAD', short: (short-of 'HEAD'), grounded: false, how: 'head'}
                }
            }
        }
    }
}

# The ONE crate-extraction regex: a workspace crate path under
# lib/{apps,components,foundation}/. Null off the workspace layout (a
# bare directory prefix without a crate segment still attributes — the
# original sites' shape, kept).
export def crate-of-path [p: string]: nothing -> any {
    $p | parse --regex '^lib/(?:apps|components|foundation)/(?P<crate>[^/]+)/' | get -o crate.0
}

# Unique crates touched by a path list, in first-appearance order.
# `parse` yields a table PER input string, so `each` would nest the
# result (list<list<record>> — the run-1 gate crash); the per-path
# helper flattens that away.
export def crates-from-paths [paths: list<string>]: nothing -> list<string> {
    $paths | each {|p| crate-of-path $p} | where {|c| $c != null} | uniq
}

# A test file by the repo's layout: tests/ dirs, *_tests.rs companions,
# or a tests.rs module file inside src/.
export def is-test-file [p: string]: nothing -> bool {
    ($p | str contains '/tests/') or ($p | str ends-with '_tests.rs') or ($p | str ends-with '/tests.rs')
}

# The touched-surface enumeration (work#1482): git diff covers tracked
# files only, so a brand-new file was invisible until staged — its crate
# dropped out of the derived verification lane and the new tests never
# ran. The untracked set (gitignore-respecting) joins the diff; the
# union is the touched surface. Over-detection is fail-safe: it widens
# verification, never narrows it.
export def diff-paths [base: string]: nothing -> list<string> {
    let tracked = (git diff --name-only $base | lines | compact)
    let untracked = (git ls-files --others --exclude-standard | lines | compact)
    $tracked | append $untracked | uniq
}

# fabro-2f70: a diff touching an inline `#[cfg(test)]` module (the repo's
# dominant test convention) also marks the file test-touched — previously
# only test-path filenames counted, so inline-module edits silently
# degraded verify to a compile check. Two signals from the per-file diff,
# both fail-safe (over-detection only widens to the full crate suite):
#   1. a CHANGED line (+/-, not context) contains `cfg(test)` — the
#      attribute itself was added/removed (whole-file deletions land here
#      too, since removing the file removes the attribute line);
#   2. a changed line falls inside the file's cfg(test) region: at/after
#      the first `#[cfg(test)]` attribute line. The region extends toward
#      EOF (approximation: content after a closed mid-file module counts
#      too), which only widens verification, never narrows it. Added
#      lines are judged in new-file numbering against the current file;
#      removed lines in old-file numbering against the base blob.
export def first-cfg-line [content: list<string>]: nothing -> any {
    let idx = ($content
        | enumerate
        | where {|r| $r.item | str contains '#[cfg(test)]'}
        | get -o index)
    if ($idx | is-empty) { null } else { ($idx | first) + 1 } # 1-based
}

export def is-inline-test-diff [base: string, p: string]: nothing -> bool {
    let old_blob = (do { git show $"($base):($p)" } | complete)
    # NEW-FILE widening (work#1482): git diff is empty for untracked
    # paths, so a brand-new source file cannot prove its inline-test
    # content through the diff. A path that exists now but has no base
    # blob is new; its own cfg(test) content decides (fail-safe:
    # over-detection widens to the full crate suite, never narrows).
    if ($old_blob.exit_code != 0) and ($p | path exists) {
        return ((open --raw $p | lines) | any {|l| $l | str contains '#[cfg(test)]'})
    }
    let diff = (git diff $base -- $p)
    if ($diff | is-empty) { return false }
    let lines = ($diff | lines)
    let changed = ($lines | where {|l|
        ((($l | str starts-with '+') and (not ($l | str starts-with '+++'))) or
            (($l | str starts-with '-') and (not ($l | str starts-with '---'))))})
    if ($changed | any {|l| $l | str contains 'cfg(test)'}) { return true }
    if (not ($p | path exists)) { return false }
    let new_cfg = (first-cfg-line (open --raw $p | lines))
    let old_cfg = (if $old_blob.exit_code == 0 {
        first-cfg-line ($old_blob.stdout | lines)
    } else { null })
    mut old_ln = 0
    mut new_ln = 0
    for l in $lines {
        if ($l | str starts-with '@@') {
            let h = ($l | parse --regex '^@@ -(?P<o>\d+)(?:,\d+)? \+(?P<n>\d+)(?:,\d+)? @@')
            if ($h | is-not-empty) {
                $old_ln = ($h.o | first | into int)
                $new_ln = ($h.n | first | into int)
            }
        } else if ($l | str starts-with '+') and (not ($l | str starts-with '+++')) {
            if ($new_cfg != null) and ($new_ln >= $new_cfg) { return true }
            $new_ln = $new_ln + 1
        } else if ($l | str starts-with '-') and (not ($l | str starts-with '---')) {
            if ($old_cfg != null) and ($old_ln >= $old_cfg) { return true }
            $old_ln = $old_ln + 1
        } else if ($l | str starts-with ' ') {
            $old_ln = $old_ln + 1
            $new_ln = $new_ln + 1
        }
        # headers, \-no-newline markers, and everything else: no advance
    }
    false
}

# The web surface (work#1304): the bun workspace members —
# apps/fabro-web/** and lib/packages/** (the API client is
# workspace:*-consumed by the web app, so its diffs verify through the
# web suite too).
export def web-surface-path? [p: string]: nothing -> bool {
    ($p | str starts-with 'apps/fabro-web/') or ($p | str starts-with 'lib/packages/')
}

# The vendor surface (denkhaus/work#1444): everything under vendor/ — the
# vendored fork snapshots (vendor/{petri,pebble,sandbox-driver}) AND the
# provenance record vendor/PINS.toml. A diff here must run the ADR-0034
# provenance verification — qualitygate.nu's vendor tier keys on this
# predicate and gates `just vendor-pins`.
export def vendor-surface-path? [p: string]: nothing -> bool {
    $p | str starts-with 'vendor/'
}

# The clap surface (denkhaus/work#1429): a crate's arg-surface source
# files — args.rs (the clap Parser/Subcommand definitions) and main.rs
# (the binary entry whose --help output the help snapshots pin). A diff
# touching either can change ANY command's help output, so the owning
# crate's help-snapshot-bearing tests must join its nextest scope: an
# args.rs-only diff used to verify as a bare compile check, and a stale
# help snapshot landed silently (the work#1361 finding).
export def clap-surface-path? [p: string]: nothing -> bool {
    ($p | str ends-with '/args.rs') or ($p | str ends-with '/main.rs')
}

# Pure: whether THIS crate's touched paths include a clap-surface file
# (per-crate attribution through the shared crate extraction; paths
# outside the workspace segments never attribute).
export def clap-surface-touched? [paths: list<string>, crate: string]: nothing -> bool {
    $paths | any {|p| ((crate-of-path $p) == $crate) and (clap-surface-path? $p)}
}

# Pure: the help-snapshot-bearing test names for a crate's
# `tests/it/cmd/` files. The path shape (binary `it`, module `cmd`) names
# the nextest test `cmd::<stem>::help`; a file whose source declares no
# `fn help()` test carries no help snapshot and yields no name (the
# `()` suffix keeps a `fn helper…` declaration from matching). Inputs are
# paths plus their sources, so the mapping stays battery-testable
# without a checkout. Exact full names, in input order.
export def help-snapshot-names [cmd_files: table]: nothing -> list<string> {
    $cmd_files
    | where {|r| $r.src | str contains 'fn help()'}
    | each {|r|
        let stem = ($r.file | parse --regex 'cmd/(?P<stem>[^/]+)\.rs$' | get -o stem.0)
        if $stem == null { '' } else { ('cmd::' + $stem + '::help') }
    }
    | where {|n| $n != ''}
}

# The repo root through the module's own location (scripts/lib/ is two
# levels down): makes the scope walk CWD-independent for every consumer.
const REPO_ROOT = (path self | path dirname | path dirname | path dirname)

# The ONE rule site (denkhaus/work#1429): a crate's help-snapshot scope
# from its touched paths. [] when the diff does not touch the crate's
# clap surface OR the crate carries no help-snapshot-bearing tests —
# crates without a tests/it/cmd/ family map to [] (a no-op, never a
# zero-match filter; the zero-selection guard keeps its meaning).
export def crate-help-snapshot-scope [touched: list<string>, crate: string]: nothing -> list<string> {
    if (not (clap-surface-touched? $touched $crate)) { return [] }
    let files = (glob ($REPO_ROOT | path join 'lib' '*' $crate 'tests' 'it' 'cmd' '*.rs'))
    if ($files | is-empty) { return [] }
    help-snapshot-names ($files | each {|f| ({file: $f, src: (open --raw $f)}) })
}

# Classification of a nextest display (work#1424 / work#1432). Parse the
# COMBINED stdout+stderr text — nextest prints its ENTIRE display,
# including the Summary line, to STDERR, and a stdout-only parse
# false-FAILs green name-filter runs (0 bytes on stdout). Kinds:
#   'ran'          — a Summary line with a positive tests-run count;
#   'zero-matched' — a Summary line reporting 0 tests run: the filter
#                    matched nothing, a LEGIT gate FAIL (the zero-
#                    selection guard's meaning, fabro-bdff);
#   'unparseable'  — no Summary line at all: a tooling error, NOT a gate
#                    verdict (the distinction work#1424/#1432 demand).
export def nextest-summary [output: string]: nothing -> record<kind: string, ran: any> {
    let m = ($output | lines | parse --regex 'Summary\s+\[[^\]]*\]\s+(?P<ran>\d+)\s+tests?\s+run' | last)
    if ($m | is-empty) {
        {kind: 'unparseable', ran: null}
    } else {
        let ran = ($m | get ran | into int)
        if $ran == 0 { {kind: 'zero-matched', ran: 0} } else { {kind: 'ran', ran: $ran} }
    }
}

# The name-filter gate verdict (fabro-bdff's zero-selection guard with
# the work#1424/#1432 distinction folded in): exit code + the COMBINED
# output + the extracted-name count -> {verdict, ran, reason}.
#   pass       — exited 0 and ran at least the expected count;
#   fail       — a REAL test verdict: non-zero exit, or the filter
#                matched fewer tests than extracted (zero-matched
#                included — "0 tests matched the filter" is a legit
#                FAIL, never green);
#   tool-error — the output has no Summary line at all: the tool broke
#                (crash, format drift); NOT a gate verdict — the caller
#                must report it with a distinct exit/detail marker,
#                never as a red test run (and never as green).
export def nextest-filter-verdict [exit_code: int, output: string, expected: int]: nothing -> record<verdict: string, ran: any, reason: string> {
    let s = (nextest-summary $output)
    if $s.kind == 'unparseable' {
        {verdict: 'tool-error', ran: null, reason: 'no Summary line in the nextest output'}
    } else if $exit_code != 0 {
        {verdict: 'fail', ran: $s.ran, reason: 'nextest exited non-zero'}
    } else if $s.ran < $expected {
        if $s.kind == 'zero-matched' {
            {verdict: 'fail', ran: $s.ran, reason: '0 tests matched the filter'}
        } else {
            let ran_s = ($s.ran | into string)
            let exp_s = ($expected | into string)
            {verdict: 'fail', ran: $s.ran, reason: ('ran ' + $ran_s + ' tests, extracted ' + $exp_s)}
        }
    } else {
        {verdict: 'pass', ran: $s.ran, reason: ''}
    }
}
