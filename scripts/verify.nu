#!/usr/bin/env nu
# Deterministic verification dispatcher (fabro-6e7f, user directive
# 2026-09-14): ONE entry point that decides WHICH checks to run from the
# calling agent's workflow STEP plus the current diff — replacing the
# prompt-encoded scope rules that drifted (fabro-0d56, 01M20T9S8).
#
# Stages:
#   implementer — crate-scoped fmt + clippy (default features, -D warnings)
#                 on every code-touched crate; for test-file-touched crates
#                 the new tests BY NAME FILTER when the diff only ADDS test
#                 files, the FULL crate suite when existing tests are edited
#                 (or name extraction comes up empty — fail-safe); a compile
#                 check for code-touched-only crates. Never the gate, never
#                 the workspace suite (the deterministic tester owns both).
# Exit 0 = all checks green. First red stops (qualitygate style:
# `do { ^cmd } | complete`).
#
# Touched-crate + base detection reuse the qualitygate.nu pattern. The
# stage comes from the engine-injected FABRO_STAGE env (fabro-6e7f /
# fork_stage_env.rs) since v2 (fabro-a9cc); an explicit argument remains
# as a manual override.

const PINNED_TOOLCHAIN = "nightly-2026-04-14"

# fabro-f750: FAIL paths print a bounded TAIL of the captured output.
# Cargo emits the actual diagnostics AFTER the warning preamble, so a
# head-bound can push the error past the cut (run 01M4KZ2BCS4C: an E0004
# sat behind a 2k-char warning preamble on a FAIL clippy verdict).
def output-tail [text: string, bound: int] {
    let trimmed = ($text | str trim)
    let length = ($trimmed | str length)
    if $length <= $bound {
        $trimmed
    } else {
        $trimmed | str substring ($length - $bound)..
    }
}


# A test file by the repo's layout: tests/ dirs, *_tests.rs companions,
# or a tests.rs module file inside src/.
def is-test-file [p: string] {
    ($p | str contains '/tests/') or ($p | str ends-with '_tests.rs') or ($p | str ends-with '/tests.rs')
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
def first-cfg-line [content: list<string>] {
    let idx = ($content
        | enumerate
        | where {|r| $r.item | str contains '#[cfg(test)]'}
        | get -o index)
    if ($idx | is-empty) { null } else { ($idx | first) + 1 } # 1-based
}

def is-inline-test-diff [base: string, p: string] {
    let diff = (git diff $base -- $p)
    if ($diff | is-empty) { return false }
    let lines = ($diff | lines)
    let changed = ($lines | where {|l|
        ((($l | str starts-with '+') and (not ($l | str starts-with '+++'))) or
            (($l | str starts-with '-') and (not ($l | str starts-with '---'))))})
    if ($changed | any {|l| $l | str contains 'cfg(test)'}) { return true }
    if (not ($p | path exists)) { return false }
    let new_cfg = (first-cfg-line (open --raw $p | lines))
    let old_blob = (do { git show $"($base):($p)" } | complete)
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

def current-branch [] {
    (git branch --show-current | str trim)
}

def run-base [] {
    let run_id = (
        current-branch
        | parse --regex 'fabro/run/(?P<id>[^/]+)$'
        | get -o id.0
        | default '')
    if ($run_id | is-not-empty) {
        let subjects = (git log --format='%H %s' -n 40)
        let checkpoint = ($subjects | lines | where {|l|
            # Plain-string concatenation, NOT $'...' interpolation: an
            # interpolated string parses every (...) — including the regex
            # group (?P<sha>...) — as a subexpression (parse bug found by
            # run 01M2GVW7GGGB's implementer; lint-nu guards the class).
            ($l | parse --regex ('^(?P<sha>[0-9a-f]+) fabro\(' + $run_id + '\): ') | get -o sha.0 | is-not-empty)
        })
        if ($checkpoint | is-not-empty) {
            let sha = ($checkpoint | first | parse --regex '^(?P<sha>[0-9a-f]+)' | get sha.0)
            {base: (git rev-parse $"($sha)^"), hit: true}
        } else {
            {base: (git rev-parse HEAD), hit: false}
        }
    } else {
        # Host / non-run checkout: diff against origin's branch tip when
        # available, else HEAD (no uncommitted-work signal on a clean tree).
        let br = (current-branch)
        let origin = $"origin/($br)"
        let has_origin = (do { git rev-parse --verify --quiet $origin } | complete | get exit_code) == 0
        if $has_origin {
            {base: (git rev-parse $origin), hit: true}
        } else {
            {base: (git rev-parse HEAD), hit: false}
        }
    }
}

# Crates touched by the diff, split into {code: [...], tests: [...]}:
# `tests` = crates with at least one test-file change, `code` = every
# touched crate (tests included — fmt/clippy cover them too).
def touched [] {
    let run = (run-base)
    let base = $run.base
    let paths = (git diff --name-only $base | lines | compact)
    let crate_paths = ($paths | where {|p| $p | str starts-with 'lib/' })
    let crates = ($crate_paths
        | each {|p| $p | parse --regex '^lib/(?:apps|components|foundation)/(?P<crate>[^/]+)/' }
        | flatten
        | get -o crate
        | uniq
        | compact)
    let test_files = ($crate_paths
        | where {|p| (is-test-file $p) or (is-inline-test-diff $base $p) })
    let test_crates = ($test_files
        | each {|p| $p | parse --regex '^lib/(?:apps|components|foundation)/(?P<crate>[^/]+)/' }
        | flatten
        | get -o crate
        | uniq
        | compact)
    {base: $base, code: $crates, tests: $test_crates, test_files: $test_files}
}

# Test function names declared under a test attribute in a file's Rust
# source: the attribute line (#[test], #[tokio::test(...)], #[e2e_test...],
# ...) sets a pending flag; the immediately following `fn <name>` captures
# the name. Extraction-only heuristic — an empty result never narrows
# verification (the caller falls back to the full crate suite).
def test-names [content: string] {
    mut names = []
    mut pending = false
    for l in ($content | lines) {
        let t = ($l | str trim)
        if ($t | str starts-with '#[') {
            if ($t | str contains 'test') { $pending = true }
        } else if $pending {
            let m = ($t | parse --regex '^(?:pub\s+)?(?:const\s+)?(?:async\s+)?fn\s+(?P<name>[A-Za-z0-9_]+)')
            if ($m | is-not-empty) {
                $names = ($names | append $m.name.0)
                $pending = false
            } else if ($t | str starts-with 'fn') or ($t | str starts-with 'pub fn') {
                $pending = false
            }
        }
    }
    $names
}

# Per-crate nextest scope (fabro-7f58): when EVERY test-touched file in the
# crate is purely added (absent from the base, present in the worktree),
# run only the extracted test names; any file that edits existing tests
# (present in base — modified or deleted) widens to the full crate suite.
# Fail-safe on both edges: no extractable names and unclassifiable files
# both widen to the full suite.
def test-mode-for [base: string, files: list<string>] {
    mut added = []
    for p in $files {
        let old = (do { git show $"($base):($p)" } | complete)
        if $old.exit_code == 0 {
            return {mode: 'full', reason: $"($p) edits existing tests"}
        }
        if (not ($p | path exists)) {
            return {mode: 'full', reason: $"($p) deleted"}
        }
        $added = ($added | append $p)
    }
    mut names = []
    for p in $added {
        $names = ($names | append (test-names (open --raw $p)))
    }
    let flat = ($names | flatten | uniq | compact)
    if ($flat | is-empty) { return {mode: 'full', reason: 'no test names extracted'} }
    {mode: 'names', names: $flat}
}

def stage-implementer [] {
    let t = (touched)
    if ($t.code | is-empty) {
        print "verify: no lib/ crates touched — nothing to verify (parse-level checks belong to the brief)"
        return
    }
    print $"verify: code-touched=($t.code | str join ', ') test-file-touched=($t.tests | str join ', ')"
    mut failed = false
    for c in $t.code {
        print $"== fmt -p ($c) =="
        let res = (do { ^cargo $'+($PINNED_TOOLCHAIN)' fmt -p $c --check } | complete)
        if $res.exit_code != 0 {
            print $res.stderr
            print $"verify: FAIL fmt ($c)"
            $env.LAST_VERIFY_FAIL = "fmt"
            return
        }
        print $"verify: PASS fmt ($c)"
    }
    for c in $t.code {
        let label = ("== clippy -p " + $c + " (default features, deny warnings) ==")
        print $label
        let res = (do { ^cargo $'+($PINNED_TOOLCHAIN)' clippy -p $c --all-targets -- -D warnings } | complete)
        if $res.exit_code != 0 {
            print (output-tail $res.stderr 2000)
            print $"verify: FAIL clippy ($c)"
            return
        }
        print $"verify: PASS clippy ($c)"
    }
    let code_only = ($t.code | where {|c| $c not-in $t.tests })
    for c in $code_only {
        print $"== compile check -p ($c) =="
        let res = (do { ^cargo check -p $c --quiet } | complete)
        if $res.exit_code != 0 {
            print (output-tail $res.stderr 2000)
            print $"verify: FAIL compile ($c)"
            return
        }
        print $"verify: PASS compile ($c)"
    }
    for c in $t.tests {
        let files = ($t.test_files
            | where {|p| (($p | parse --regex '^lib/(?:apps|components|foundation)/(?P<crate>[^/]+)/') | get -o crate.0) == $c })
        let tm = (test-mode-for $t.base $files)
        if $tm.mode == 'names' {
            print $"== nextest -p ($c) (added tests by name filter: ($tm.names | str join ', ')) =="
            let res = (do { ^cargo nextest run -p $c --no-fail-fast ...$tm.names } | complete)
            if $res.exit_code != 0 {
                print (output-tail ($res.stdout + $res.stderr) 3000)
                print $"verify: FAIL nextest ($c)"
                return
            }
            print $"verify: PASS nextest ($c)"
        } else {
            print $"== nextest -p ($c) (full crate suite: ($tm.reason)) =="
            let res = (do { ^cargo nextest run -p $c --no-fail-fast } | complete)
            if $res.exit_code != 0 {
                print (output-tail ($res.stdout + $res.stderr) 3000)
                print $"verify: FAIL nextest ($c)"
                return
            }
            print $"verify: PASS nextest ($c)"
        }
    }
    print "verify: implementer stage green"
}

# Stage source (fabro-a9cc, v2): the engine-injected FABRO_STAGE env is
# authoritative inside a run (fork_stage_env.rs, fabro-6e7f); an explicit
# argument overrides it for local/manual use only.
def main [stage?: string] {
    let stage = (if ($stage == null or $stage == '') { null } else { $stage })
    let resolved = (
        if $stage != null {
            let env_stage = ($env | get -o FABRO_STAGE | default '')
            if ($env_stage != '') and ($env_stage != $stage) {
                print $"verify: WARN argument '($stage)' overrides injected FABRO_STAGE '($env_stage)' — manual use only"
            }
            $stage
        } else if (($env | get -o FABRO_STAGE | default '') != '') {
            $env.FABRO_STAGE
        } else {
            print 'verify: no stage — pass one (e.g. `just verify implementer`) or run inside a stage (FABRO_STAGE injected)'
            exit 2
        }
    )
    let src = (if $stage != null { 'argument' } else { 'FABRO_STAGE' })
    print $"verify: stage ($resolved) from ($src)"
    match $resolved {
        'implementer' => { stage-implementer }
        _ => {
            print $"verify: unknown stage '($resolved)' — v2 supports implementer"
            exit 2
        }
    }
}
