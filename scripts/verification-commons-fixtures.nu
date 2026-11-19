#!/usr/bin/env nu
# Verification-commons battery (denkhaus/work#1445, architecture
# candidate C1): pins scripts/lib/verification_commons.nu — the ONE
# diff-scope seam for verify.nu, qualitygate.nu and run_anchor.nu
# (work#1108's extraction pattern, folded wider).
#
# Pin families:
#   1. PURE pins — the run-id parse, the subject mark, crate extraction,
#      test-file classification, the web and vendor predicates, the
#      nextest Summary
#      parse from COMBINED streams (work#1424: the Summary rides stderr),
#      and the work#1424/#1432 verdict split ("0 tests matched the
#      filter" = legit FAIL vs "output unparseable" = tooling error,
#      never a gate verdict).
#   2. GIT-FIXTURE pins — scratch repos: checkpoint hit (earliest and
#      latest pick), the checkpoint-miss JOURNAL fallback (work#1427:
#      base = the last journal commit's parent, NOT unconditional HEAD^),
#      the head-parent last resort, the root edge, the off-run HEAD
#      contract through run_anchor's adapter (its public record shape
#      unchanged), and the inline-cfg(test) hunk mapping (fabro-2f70).
#   3. REGROWTH pins — no mirror def/regex regrows beside the module in
#      verify.nu, qualitygate.nu or run_anchor.nu; run_anchor's defs all
#      delegate; verify.nu feeds the COMBINED streams to the verdict.
#
# Every family was proven RED FIRST against the pre-fix behavior (the
# observations are recorded in the work#1445 close comment): pre-fix
# parser/behavior restored -> pin red -> restored -> green.

use lib/verification_commons.nu
use ../.fabro/scripts/lib/run_anchor.nu

const DIR = (path self | path dirname)
let VERIFY_SRC = (open --raw ($DIR | path join 'verify.nu'))
let QUALITYGATE_SRC = (open --raw ($DIR | path join 'qualitygate.nu'))
let RUN_ANCHOR_SRC = (open --raw ($DIR | path join '../.fabro/scripts/lib/run_anchor.nu'))

def fail [what: string]: nothing -> nothing {
    print -e $"verification-commons: FAIL — ($what)"
    exit 1
}

def assert-eq [got: any, want: any, what: string]: nothing -> nothing {
    if $got != $want {
        print -e $"verification-commons: ($what): got ($got | to json --raw), want ($want | to json --raw)"
        exit 1
    }
}

def assert-ne [got: any, not_want: any, what: string]: nothing -> nothing {
    if $got == $not_want {
        print -e $"verification-commons: ($what): must NOT be ($not_want | to json --raw), but is"
        exit 1
    }
}

def assert-contains [haystack: string, needle: string, what: string]: nothing -> nothing {
    if not ($haystack | str contains $needle) {
        print -e $"verification-commons: ($what): missing `($needle)`"
        exit 1
    }
}

# --- 1. pure pins: the anchor triple ---------------------------------------

def test-run-id-parse []: nothing -> nothing {
    assert-eq (verification_commons id-from-branch 'fabro/run/01M5844J1FDD') '01M5844J1FDD' 'run branch yields the id'
    assert-eq (verification_commons id-from-branch 'main') '' 'off-run default is empty'
    assert-eq (verification_commons id-from-branch 'fabro/run/a/b') '' 'nested path under fabro/run is off-run'
    assert-eq (verification_commons id-from-branch '') '' 'detached HEAD is off-run'
    # run_anchor's adapter: same answers through the unchanged public API.
    # (The ambient `run_anchor run-id` assertion moved into
    # test-off-run-contract below — work#1456: it read AMBIENT checkout
    # state and can never hold inside a run sandbox; two laps paid the
    # same tester re-entry gate-red on 2026-11-18.)
    assert-eq (run_anchor id-from-branch 'fabro/run/01M5844J1FDD' 'local') '01M5844J1FDD' 'run_anchor adapter yields the id'
    assert-eq (run_anchor id-from-branch 'main' 'local') 'local' 'run_anchor adapter keeps the per-consumer off-run sentinel'
}

def test-subject-mark []: nothing -> nothing {
    # The mark carries LITERAL parentheses for --fixed-strings (the
    # escape incident: a regex-shaped mark matched nothing).
    assert-eq (verification_commons subject-mark 'fx1') 'fabro(fx1):' 'subject mark carries literal parentheses'
    assert-eq (run_anchor subject-mark 'fx1') 'fabro(fx1):' 'run_anchor adapter re-exports the mark'
}

# --- 1b. pure pins: crate extraction (one regex, four former sites) -------

def test-crate-extraction []: nothing -> nothing {
    let paths = [
        'lib/apps/fabro-cli/src/main.rs'
        'lib/components/fabro-workflow/src/lib.rs'
        'lib/components/fabro-workflow/tests/it/main.rs'
        'lib/foundation/fabro-types/src/lib.rs'
        'apps/fabro-web/src/x.ts'
        'docs/x.md'
        'Cargo.toml'
    ]
    assert-eq (verification_commons crates-from-paths $paths) [
        'fabro-cli'
        'fabro-workflow'
        'fabro-types'
    ] 'crate extraction: exactly the touched workspace crates, first-appearance order'
    assert-eq (verification_commons crate-of-path 'lib/components/fabro-dot/src/lib.rs') 'fabro-dot' 'crate-of-path yields the crate'
    assert-eq (verification_commons crate-of-path 'apps/fabro-web/src/x.ts') null 'crate-of-path: non-lib path is null'
    assert-eq (verification_commons crate-of-path 'lib/nonworkspace/x.rs') null 'crate-of-path: outside the workspace segments is null'
}

# --- 1c. pure pins: test-file classification + web predicate ---------------

def test-test-file-classification []: nothing -> nothing {
    assert-eq (verification_commons is-test-file 'lib/components/fabro-dot/tests/it/main.rs') true 'tests/ dir is a test file'
    assert-eq (verification_commons is-test-file 'lib/apps/fabro-cli/src/complete_tests.rs') true '*_tests.rs companion is a test file'
    assert-eq (verification_commons is-test-file 'lib/components/fabro-dot/src/tests.rs') true 'src/tests.rs module file is a test file'
    assert-eq (verification_commons is-test-file 'lib/apps/fabro-cli/src/main.rs') false 'src/main.rs is not a test file'
    assert-eq (verification_commons is-test-file 'lib/apps/fabro-cli/src/testing.rs') false 'a testing-path is not a test file (dir-anchored check)'
}

def test-web-predicate []: nothing -> nothing {
    assert-eq (verification_commons web-surface-path? 'apps/fabro-web/src/x.ts') true 'web app path is the web surface'
    assert-eq (verification_commons web-surface-path? 'lib/packages/fabro-api-client/src/x.ts') true 'lib/packages path is the web surface (workspace:*-consumed)'
    assert-eq (verification_commons web-surface-path? 'lib/components/fabro-dot/src/lib.rs') false 'lib/components is not the web surface'
    assert-eq (verification_commons web-surface-path? 'apps/marketing/x.ts') false 'other apps are not the web surface'
}

# The vendor surface (denkhaus/work#1444): everything under vendor/ —
# the vendored fork snapshots AND vendor/PINS.toml — must route the diff
# to the qualitygate's vendor tier (`just vendor-pins`). Workspace-crate
# paths (lib/...) never fire it, and a vendor-prefixed sibling directory
# (vendorx/) is not the surface either.
def test-vendor-predicate []: nothing -> nothing {
    assert-eq (verification_commons vendor-surface-path? 'vendor/petri/Cargo.toml') true 'a vendored fork path is the vendor surface'
    assert-eq (verification_commons vendor-surface-path? 'vendor/PINS.toml') true 'the PINS provenance file itself is the vendor surface'
    assert-eq (verification_commons vendor-surface-path? 'lib/apps/fabro-cli/src/main.rs') false 'lib/apps is not the vendor surface'
    assert-eq (verification_commons vendor-surface-path? 'lib/components/fabro-dot/src/lib.rs') false 'lib/components is not the vendor surface'
    assert-eq (verification_commons vendor-surface-path? 'vendorx/petri/x') false 'a vendor-prefixed sibling directory is not the vendor surface'
}

# --- 1e. pure pins: the clap-surface -> help-snapshot rule (work#1429) ------
#
# The defect (the work#1361 finding): a clap-surface edit that changes
# command help output mapped to a CODE-ONLY crate in verify's touched
# slice — compile check only, the help snapshots never executed, and the
# stale snapshot reds a later full run. The rule: a crate whose diff
# touches args.rs/main.rs gains its cmd::*::help family in scope.

def test-clap-surface-predicate []: nothing -> nothing {
    assert-eq (verification_commons clap-surface-path? 'lib/apps/fabro-cli/src/args.rs') true 'args.rs is the clap surface'
    assert-eq (verification_commons clap-surface-path? 'lib/apps/fabro-cli/src/main.rs') true 'main.rs is the clap surface'
    assert-eq (verification_commons clap-surface-path? 'lib/apps/fabro-cli/src/args_tests.rs') false 'args_tests.rs is not the clap surface'
    assert-eq (verification_commons clap-surface-path? 'lib/apps/fabro-cli/src/command_context.rs') false 'a non-surface source file is not the clap surface'
    # Per-crate attribution through the shared crate extraction.
    let paths = ['lib/apps/fabro-cli/src/args.rs']
    assert-eq (verification_commons clap-surface-touched? $paths 'fabro-cli') true 'the owning crate is clap-touched'
    assert-eq (verification_commons clap-surface-touched? $paths 'fabro-dot') false 'a sibling crate is not clap-touched'
    assert-eq (verification_commons clap-surface-touched? ['lib/apps/fabro-cli/src/lib.rs'] 'fabro-cli') false 'a non-surface touch does not fire the rule'
    assert-eq (verification_commons clap-surface-touched? ['vendor/petri/src/args.rs'] 'petri') false 'outside the workspace segments never attributes'
}

def test-help-snapshot-names []: nothing -> nothing {
    let files = [
        {file: 'lib/apps/fabro-cli/tests/it/cmd/fabro.rs', src: "#[test]\nfn help() { fabro_snapshot!(...) }"}
        {file: 'lib/apps/fabro-cli/tests/it/cmd/ps.rs', src: "#[test]\nfn help() { ... }\n#[test]\nfn other() { ... }"}
        {file: 'lib/apps/fabro-cli/tests/it/cmd/dump.rs', src: "#[test]\nfn dump_contents() { ... }"}
        {file: 'lib/apps/fabro-cli/tests/it/cmd/wait.rs', src: "#[test]\nfn helper_setup() { ... }"}
    ]
    assert-eq (verification_commons help-snapshot-names $files) [
        'cmd::fabro::help'
        'cmd::ps::help'
    ] 'help-snapshot names derive as cmd::<stem>::help; help-less files yield nothing'
    assert-eq (verification_commons help-snapshot-names []) [] 'no cmd files -> no names'
}

def test-help-snapshot-scope-mapping []: nothing -> nothing {
    # THE RULE (work#1429): a clap-surface touch in a crate's diff puts
    # that crate's cmd::*::help family in scope. Real-tree arm: fabro-cli
    # owns tests/it/cmd/ (binary `it`, module `cmd`).
    let scope = (verification_commons crate-help-snapshot-scope ['lib/apps/fabro-cli/src/args.rs'] 'fabro-cli')
    if ($scope | is-empty) {
        fail 'work#1429 rule: an args.rs diff maps fabro-cli to a NON-EMPTY help-snapshot scope'
    }
    if (not ($scope | any {|n| ($n | str starts-with 'cmd::') and ($n | str ends-with '::help')})) {
        fail $"work#1429 rule: every scope entry must name a cmd::*::help test, got ($scope | to json --raw)"
    }
    assert-eq $scope (verification_commons crate-help-snapshot-scope ['lib/apps/fabro-cli/src/main.rs'] 'fabro-cli') 'the scope is deterministic across clap-surface files'
    assert-eq (verification_commons crate-help-snapshot-scope ['lib/apps/fabro-cli/src/command_context.rs'] 'fabro-cli') [] 'a non-clap diff adds no scope'
    # A clap-surface crate WITHOUT the cmd test family maps to [] — a
    # no-op, never a zero-match filter (the zero-selection guard keeps
    # its meaning, fabro-bdff).
    assert-eq (verification_commons crate-help-snapshot-scope ['lib/foundation/fabro-types/src/main.rs'] 'fabro-types') [] 'a crate without cmd tests adds no scope'
}

def test-first-cfg-line []: nothing -> nothing {
    let content = [
        'use std::fmt;'
        ''
        'pub struct X;'
        '#[cfg(test)]'
        'mod tests {'
        '    #[test]'
        '    fn t() {}'
        '}'
    ]
    assert-eq (verification_commons first-cfg-line $content) 4 'cfg(test) line number is 1-based'
    assert-eq (verification_commons first-cfg-line ['no tests here']) null 'no cfg(test) -> null'
}

# --- 1d. pure pins: Summary parse + the verdict split -----------------------

def test-summary-parse-combined []: nothing -> nothing {
    # The work#1424 evidence shape: 0 bytes on stdout, the full display —
    # Summary line included — on stderr. The parser takes the COMBINED text.
    let stdout = ''
    let stderr = "------------\n------------\n Summary [  1.234s] 12 tests run: 12 passed, 0 failed, 0 skipped\n"
    let s = (verification_commons nextest-summary ($stdout + $stderr))
    assert-eq $s.kind 'ran' 'combined streams parse (stdout empty, Summary rides stderr — work#1424)'
    assert-eq $s.ran 12 'ran count extracted from the display shape'
    let s2 = (verification_commons nextest-summary $stderr)
    assert-eq $s2.kind 'ran' 'a stderr-only display parses the same way'
    # zero-matched: the Summary exists and reports 0 — a LEGIT FAIL, not
    # unparseable (the distinction work#1424/#1432 demand).
    let zero = " Summary [  0.001s] 0 tests run: 0 passed, 0 failed, 0 skipped\n"
    assert-eq (verification_commons nextest-summary $zero).kind 'zero-matched' 'a 0-tests Summary classifies zero-matched'
    # unparseable: no Summary line at all.
    let broken = "Starting 12 tests across 3 binaries\nthread 'main' panicked\n"
    assert-eq (verification_commons nextest-summary $broken).kind 'unparseable' 'no Summary line is unparseable'
    # Multiple Summary lines: the LAST wins (the current parser's shape).
    let two = " Summary [  1.000s] 5 tests run: 5 passed\n Summary [  2.000s] 9 tests run: 9 passed\n"
    assert-eq (verification_commons nextest-summary $two).ran 9 'multiple Summary lines: the last wins'
}

def test-verdict-split []: nothing -> nothing {
    let combined = "------------\n Summary [  1.234s] 12 tests run: 12 passed, 0 failed, 0 skipped\n"
    let zero = " Summary [  0.001s] 0 tests run: 0 passed, 0 failed, 0 skipped\n"
    let broken = "thread 'main' panicked at nextest\n"
    assert-eq (verification_commons nextest-filter-verdict 0 $combined 12).verdict 'pass' 'green slice passes (ran >= extracted)'
    # "0 tests matched the filter": a LEGIT FAIL — the zero-selection
    # guard keeps its meaning.
    assert-eq (verification_commons nextest-filter-verdict 0 $zero 12).verdict 'fail' 'zero-matched is a legit FAIL'
    assert-eq (verification_commons nextest-filter-verdict 0 $zero 12).reason '0 tests matched the filter' 'the reason names the matched-nothing case'
    # A real red: non-zero exit with a parsed summary.
    assert-eq (verification_commons nextest-filter-verdict 1 $combined 12).verdict 'fail' 'non-zero exit is a FAIL'
    # Extraction mismatch: ran fewer than extracted.
    assert-eq (verification_commons nextest-filter-verdict 0 $combined 15).verdict 'fail' 'ran < extracted is a FAIL'
    # THE DISTINCTION (work#1424/#1432): unparseable output is a tooling
    # error, classified differently from any FAIL, regardless of exit code.
    let v = (verification_commons nextest-filter-verdict 1 $broken 12)
    assert-eq $v.verdict 'tool-error' 'unparseable output is a tool-error (non-zero exit)'
    assert-ne $v.verdict 'fail' 'tool-error is NOT a fail verdict'
    assert-eq (verification_commons nextest-filter-verdict 0 $broken 12).verdict 'tool-error' 'exit 0 with no Summary is still a tool-error (never looks green)'
}

# --- 2. git-fixture pins ----------------------------------------------------
#
# CD-SCOPING LESSON (observed live 2026-11-18, the first battery drive):
# `cd` inside a nu def is SCOPE-LOCAL — it reverts when the def returns.
# A scratch-repo helper must not `cd` for the caller: every fixture def
# does its own `cd $dir` and holds that scope for ALL of its module calls
# (run-base / is-inline-test-diff run CWD-relative git) and its raw git
# calls, then returns to $DIR and removes the scratch tree. A helper's cd
# silently reverting pointed the first drive's git commands at the fabro
# worktree itself — the hooks dutifully committed them.

# Scratch repo scaffold: one base commit on main. Returns the dir; the
# FIXTURE def cd's into it (never the helper — see the lesson above).
def scratch-repo []: nothing -> string {
    let dir = (mktemp -d)
    do { ^git -C $dir init -q -b main } | ignore
    ^git -C $dir config user.email fx@fabro.sh
    ^git -C $dir config user.name fx
    'base' | save ($dir | path join 'base.txt')
    do { ^git -C $dir add -A } | ignore
    do { ^git -C $dir commit -qm base } | ignore
    $dir
}

# An engine-written journal/auto commit: non-checkpoint subject, the
# `Fabro-Run: <id>` trailer (see any merge commit by fabro-the-factory).
# Runs INSIDE the fixture def's cd scope.
def journal-commit [msg: string, run_id: string, file: string, content: string]: nothing -> nothing {
    $content | save $file
    do { ^git add -A } | ignore
    let trailer = ('Fabro-Run: ' + $run_id)
    do { ^git commit -qm $msg -m $trailer } | ignore
}

def test-checkpoint-hit []: nothing -> nothing {
    let dir = (scratch-repo)
    cd $dir
    do { ^git checkout -qb fabro/run/fxc1 } | ignore
    'one' | save a.txt
    do { ^git add -A } | ignore
    do { ^git commit -qm 'fabro(fxc1): implementer (success)' } | ignore
    'two' | save b.txt
    do { ^git add -A } | ignore
    do { ^git commit -qm 'fabro(fxc1): tester (success)' } | ignore
    # earliest pick (the gate/run_anchor contract): the FIRST checkpoint's
    # parent — the whole-run scope.
    let e = (verification_commons run-base '' --checkpoint-pick 'earliest')
    assert-eq $e.how 'checkpoint' 'checkpoint hit names its arm'
    assert-eq $e.grounded true 'checkpoint hit is grounded'
    assert-eq $e.base (git rev-parse 'HEAD~1^') 'earliest pick = first checkpoint parent (whole-run scope)'
    # latest pick (verify.nu's contract): the newest checkpoint's parent.
    let l = (verification_commons run-base '' --checkpoint-pick 'latest')
    assert-eq $l.base (git rev-parse 'HEAD^') 'latest pick = newest checkpoint parent (stage scope)'
    # run_anchor's adapter: earliest by default, record shape unchanged.
    let ra = (run_anchor run-base)
    assert-eq $ra.base $e.base 'run_anchor adapter matches the earliest pick'
    assert-eq ($ra | columns | sort) [base grounded short] 'run_anchor run-base record shape unchanged'
    cd $DIR
    rm -rf $dir
}

def test-journal-fallback []: nothing -> nothing {
    let dir = (scratch-repo)
    cd $dir
    do { ^git checkout -qb fabro/run/fxj1 } | ignore
    # Engine journal/auto commits: checkpoint-miss subjects, run trailers.
    journal-commit 'auto: wip savepoint' 'fxj1' 'a.txt' 'journal one'
    journal-commit 'auto: wip savepoint two' 'fxj1' 'b.txt' 'journal two'
    # A shell-written fixup at HEAD: no trailer, no checkpoint subject.
    'fix' | save c.txt
    do { ^git add -A } | ignore
    do { ^git commit -qm 'manual fixup' } | ignore
    let r = (verification_commons run-base '')
    # work#1427: the fallback grounds on the NEWEST journal commit's
    # parent — "the last journal commit's diff". Unconditional HEAD^ (the
    # ac624e096a interim) would scope to the fixup alone and silently
    # skip b.txt.
    assert-eq $r.how 'journal' 'checkpoint-miss grounds on the journal commit and names the arm'
    assert-eq $r.grounded true 'journal fallback is grounded'
    assert-eq $r.base (git rev-parse 'HEAD~2') 'base = the last journal commit parent (fixup^ would be HEAD~1)'
    let scope = (git diff --name-only $r.base | lines | compact | sort)
    assert-eq $scope [b.txt c.txt] 'scope covers the journal diff plus later work (HEAD^ covers c.txt only)'
    cd $DIR
    rm -rf $dir
}

def test-head-parent-last-resort []: nothing -> nothing {
    let dir = (scratch-repo)
    cd $dir
    do { ^git checkout -qb fabro/run/fxh1 } | ignore
    'x' | save x.txt
    do { ^git add -A } | ignore
    do { ^git commit -qm 'plain work commit' } | ignore
    'y' | save y.txt
    do { ^git add -A } | ignore
    do { ^git commit -qm 'another plain commit' } | ignore
    let r = (verification_commons run-base '')
    # No checkpoint subject, no run trailer anywhere: HEAD^ is the LAST
    # resort (the ac624e096a arm), still a real diffable base.
    assert-eq $r.how 'head-parent' 'no checkpoint and no trailer commit -> head-parent last resort'
    assert-eq $r.base (git rev-parse 'HEAD^') 'last-resort base is HEAD^'
    assert-eq $r.grounded true 'last resort is grounded'
    cd $DIR
    rm -rf $dir
}

def test-root-edge []: nothing -> nothing {
    let dir = (scratch-repo)
    cd $dir
    do { ^git checkout -qb fabro/run/fxr1 } | ignore
    let r = (verification_commons run-base '')
    assert-eq $r.how 'head' 'root edge: no parent to fall back to'
    assert-eq $r.base 'HEAD' 'root edge base is HEAD'
    assert-eq $r.grounded false 'root edge is ungrounded'
    cd $DIR
    rm -rf $dir
}

def test-off-run-contract []: nothing -> nothing {
    let dir = (scratch-repo)
    cd $dir
    # NOT a run branch (main): run_anchor's contract is HEAD + ungrounded,
    # and the record keeps its exact 3-field shape.
    let ra = (run_anchor run-base)
    assert-eq $ra.base 'HEAD' 'off-run base is HEAD (the run_anchor contract)'
    assert-eq $ra.grounded false 'off-run is ungrounded'
    assert-eq ($ra | columns | sort) [base grounded short] 'off-run record shape unchanged'
    let m = (verification_commons run-base '')
    assert-eq $m.how 'off-run' 'the module names the off-run arm'
    # verify.nu's origin arm: a synthetic remote-tracking ref is enough.
    do { ^git update-ref refs/remotes/origin/main HEAD } | ignore
    let o = (verification_commons run-base '' --origin-branch)
    assert-eq $o.how 'origin' 'the origin arm names itself'
    assert-eq $o.base (git rev-parse HEAD) 'the origin arm grounds on the remote tip'
    # The run-branch kind pins here too (work#1456, 2026-11-18): the
    # ambient `run_anchor run-id` assertion this battery once carried can
    # never hold inside a run sandbox (the branch is fabro/run/<id>
    # there) — two laps paid the same tester re-entry gate-red before the
    # assertion moved into the controlled scratch.
    git checkout -q -b fabro/run/fxrid
    assert-eq (run_anchor run-id) 'fxrid' 'run branch: run_anchor run-id yields the run id'
    assert-eq (run_anchor id-from-branch 'fabro/run/fxrid' 'local') 'fxrid' 'run branch: id-from-branch yields the id'
    cd $DIR
    rm -rf $dir
}

def test-inline-cfg-hunk-mapping []: nothing -> nothing {
    let dir = (scratch-repo)
    cd $dir
    mkdir src
    let v1 = ("pub fn a() -> i32 { 1 }\n"
        + "pub fn b() -> i32 { 2 }\n"
        + "#[cfg(test)]\n"
        + "mod tests {\n"
        + "    #[test]\n"
        + "    fn t_a() { assert_eq!(a(), 1); }\n"
        + "}\n")
    $v1 | save src/lib.rs
    do { ^git add -A } | ignore
    do { ^git commit -qm base } | ignore
    let base = (git rev-parse HEAD)
    # Edit INSIDE the cfg(test) region -> test-touched (the fabro-2f70
    # hunk-mapping signal).
    let v2 = ("pub fn a() -> i32 { 1 }\n"
        + "pub fn b() -> i32 { 2 }\n"
        + "#[cfg(test)]\n"
        + "mod tests {\n"
        + "    #[test]\n"
        + "    fn t_a() { assert_eq!(a(), 2); }\n"
        + "}\n")
    $v2 | save --force src/lib.rs
    assert-eq (verification_commons is-inline-test-diff $base 'src/lib.rs') true 'a hunk inside the cfg(test) region is test-touched'
    # Edit BEFORE the region -> not test-touched (region-aware mapping).
    let v3 = ("pub fn a() -> i32 { 42 }\n"
        + "pub fn b() -> i32 { 2 }\n"
        + "#[cfg(test)]\n"
        + "mod tests {\n"
        + "    #[test]\n"
        + "    fn t_a() { assert_eq!(a(), 1); }\n"
        + "}\n")
    $v3 | save --force src/lib.rs
    assert-eq (verification_commons is-inline-test-diff $base 'src/lib.rs') false 'a hunk before the cfg(test) region is not test-touched'
    # A CHANGED cfg(test) attribute line itself -> test-touched (signal 1).
    do { ^git checkout -q -- src/lib.rs } | ignore
    let before = (open --raw src/lib.rs)
    let after = ($before | str replace '#[cfg(test)]' '#[cfg(test)] // touched')
    $after | save --force src/lib.rs
    assert-eq (verification_commons is-inline-test-diff $base 'src/lib.rs') true 'a changed cfg(test) attribute line is test-touched'
    cd $DIR
    rm -rf $dir
}

# --- 3. regrowth pins: the extraction teeth ---------------------------------

def test-consumers-source-the-module []: nothing -> nothing {
    assert-contains $VERIFY_SRC 'source lib/verification_commons.nu' 'verify.nu sources the module'
    assert-contains $QUALITYGATE_SRC 'source lib/verification_commons.nu' 'qualitygate.nu sources the module'
}

def test-no-mirror-regrows []: nothing -> nothing {
    # Any of these defs back in a consumer means a module-owned shape
    # regrew beside it (the mirrored-copies state work#1445 retired).
    let def_markers = [
        'def run-base'
        'def current-branch'
        'def nextest-tests-run'
        'def nextest-summary'
        'def nextest-filter-verdict'
        'def is-test-file'
        'def is-inline-test-diff'
        'def first-cfg-line'
        'def clap-surface-path?'
        'def clap-surface-touched?'
        'def help-snapshot-names'
        'def crate-help-snapshot-scope'
    ]
    for pair in [['verify.nu' $VERIFY_SRC] ['qualitygate.nu' $QUALITYGATE_SRC]] {
        for marker in $def_markers {
            if ($pair.1 | str contains $marker) {
                fail $"mirror regrew in scripts/($pair.0): `($marker)` — scripts/lib/verification_commons.nu owns that shape now"
            }
        }
    }
    # The literals too: one regex, one mark, one journal detector —
    # module-owned, never re-inlined (comments included: a commented-out
    # copy invites a copy-paste resurrection).
    let lit_markers = [
        'fabro/run/(?P<id>'
        '^lib/(?:apps|components|foundation)/(?P<crate>'
        'Summary\s+\[[^\]]*\]'
        'fabro\\('
        'Fabro-Run: '
        "str starts-with 'apps/fabro-web/'"
        "str starts-with 'lib/packages/'"
        "str starts-with 'vendor/'"
        "str ends-with '/args.rs'"
        "str ends-with '/main.rs'"
    ]
    for pair in [
        ['verify.nu' $VERIFY_SRC]
        ['qualitygate.nu' $QUALITYGATE_SRC]
        ['run_anchor.nu' $RUN_ANCHOR_SRC]
    ] {
        for marker in $lit_markers {
            if ($pair.1 | str contains $marker) {
                fail $"module literal regrew in scripts/($pair.0) \(or the adapter\): `($marker)` — scripts/lib/verification_commons.nu owns that shape now"
            }
        }
    }
}

def test-run-anchor-delegates []: nothing -> nothing {
    # The adapter contract (work#1108's public API, kept): all five names
    # exist, and every one of them delegates — the module name appears at
    # least once per def body.
    for marker in ['def id-from-branch' 'def current-branch' 'def run-id' 'def subject-mark' 'def run-base'] {
        if not ($RUN_ANCHOR_SRC | str contains $marker) {
            fail $"run_anchor lost its public API: `($marker)` missing"
        }
    }
    let refs = ($RUN_ANCHOR_SRC | split row 'verification_commons ' | length)
    if $refs < 6 {
        fail $"run_anchor adapter stopped delegating: ($refs - 1) module references for 5 defs"
    }
}

def test-verify-feeds-combined-streams []: nothing -> nothing {
    # The verdict call site must feed the COMBINED streams: nextest writes
    # its Summary to STDERR; a stdout-only feed false-FAILs green slices
    # (work#1424). Textual pin on the one call site.
    assert-contains $VERIFY_SRC 'nextest-filter-verdict $res.exit_code ($res.stdout + $res.stderr)' 'verify.nu feeds the combined streams to the verdict'
}

def test-verify-attaches-help-snapshot-scope []: nothing -> nothing {
    # The work#1429 rule must be CONSUMED, not just defined: verify.nu
    # derives the help-snapshot scope through the module (never re-derives
    # per-recipe), carries it on the touched record, and attaches it in
    # BOTH selection arms — the code-only compile-check lane (the defect
    # path) and the added-tests name-filter lane. The snapshot tests
    # themselves run tonight's lanes; this pins the wiring.
    assert-contains $VERIFY_SRC 'crate-help-snapshot-scope' 'verify.nu derives the help-snapshot scope through the module'
    assert-contains $VERIFY_SRC 'help_scope' 'verify.nu carries the help-snapshot scope on the touched record'
    assert-contains $VERIFY_SRC 'help-names-for' 'verify.nu attaches the scope in both selection arms'
}

def main []: nothing -> nothing {
    test-run-id-parse
    test-subject-mark
    test-crate-extraction
    test-test-file-classification
    test-web-predicate
    test-vendor-predicate
    test-clap-surface-predicate
    test-help-snapshot-names
    test-help-snapshot-scope-mapping
    test-first-cfg-line
    test-summary-parse-combined
    test-verdict-split
    test-checkpoint-hit
    test-journal-fallback
    test-head-parent-last-resort
    test-root-edge
    test-off-run-contract
    test-inline-cfg-hunk-mapping
    test-consumers-source-the-module
    test-no-mirror-regrows
    test-run-anchor-delegates
    test-verify-feeds-combined-streams
    test-verify-attaches-help-snapshot-scope
    print 'verification-commons: all pins green'
}
