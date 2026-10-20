#!/usr/bin/env nu
# Deterministic verification dispatcher (fabro-6e7f, user directive
# 2026-09-14): ONE entry point that decides WHICH checks to run from the
# calling agent's workflow STEP plus the current diff — replacing the
# prompt-encoded scope rules that drifted (fabro-0d56, 01M20T9S8).
#
# Stages:
#   implementer — crate-scoped fmt + clippy (default features, -D warnings)
#                 on every code-touched crate; the test tier is narrowed
#                 (fabro-7f58): when the diff only ADDS test files, run the
#                 new tests by name filter; the FULL crate suite only when
#                 EXISTING tests are edited; a compile check for
#                 code-touched-only crates. Never the gate, never the
#                 workspace suite (the deterministic tester owns both).
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
    let base = (run-base).base
    let paths = (git diff --name-only $base | lines | compact)
    let crate_paths = ($paths | where {|p| $p | str starts-with 'lib/' })
    let crates = ($crate_paths
        | each {|p| $p | parse --regex '^lib/(?:apps|components|foundation)/(?P<crate>[^/]+)/' }
        | flatten
        | get -o crate
        | uniq
        | compact)
    let test_crates = ($crate_paths
        | where {|p| (is-test-file $p) or (is-inline-test-diff $base $p) }
        | each {|p| $p | parse --regex '^lib/(?:apps|components|foundation)/(?P<crate>[^/]+)/' }
        | flatten
        | get -o crate
        | uniq
        | compact)
    # fabro-7f58: per-file name-status split of the test-touching paths.
    # A path is "added" only when git reports it as a new file (A);
    # modified/deleted/renamed (and anything unparseable) count as
    # edited — fail-safe toward the wider full-crate-suite tier.
    let status_rows = (git diff --name-status $base | lines | compact
        | parse --regex '^(?P<st>[A-Z])\t+(?P<path>\S+)$')
    let status_of = {|p|
        ($status_rows | where path == $p | get -o st.0 | default 'M')
    }
    let added_test_files = ($crate_paths
        | where {|p| ((is-test-file $p) or (is-inline-test-diff $base $p)) and ((do $status_of $p) == 'A') }
        | each {|p| {crate: ($p | parse --regex '^lib/(?:apps|components|foundation)/(?P<crate>[^/]+)/' | get -o crate.0), path: $p} })
    let edited_test_crates = ($crate_paths
        | where {|p| ((is-test-file $p) or (is-inline-test-diff $base $p)) and ((do $status_of $p) != 'A') }
        | each {|p| $p | parse --regex '^lib/(?:apps|components|foundation)/(?P<crate>[^/]+)/' }
        | flatten
        | get -o crate
        | uniq
        | compact)
    {code: $crates, tests: $test_crates, added_test_files: $added_test_files, edited_test_crates: $edited_test_crates}
}

# fabro-7f58: test-function names inside a newly added test file — every
# `fn name(...)` whose accumulated attribute window (the `#[...]` lines
# directly above it) contains a test-ish attribute (`#[test]`,
# `#[tokio::test]`, `#[e2e_test(...)]`, `#[rstest]`, ...). Extracted names
# feed nextest's substring filters; a file yielding no names disqualifies
# the narrowing (the caller falls back to the full crate suite).
def test-fn-names [p: string] {
    let lines = (open --raw $p | lines)
    mut attrs = []
    mut names = []
    for l in $lines {
        let t = ($l | str trim)
        if ($t | str starts-with '#[') {
            $attrs = ($attrs | append $t)
        } else {
            let m = ($t | parse --regex '^(?:pub )?(?:async unsafe |async |unsafe )?fn (?P<name>[A-Za-z0-9_]+)')
            if ($m | is-not-empty) and ($attrs | any {|a| $a | str contains 'test' }) {
                $names = ($names | append ($m | get name.0))
            }
            $attrs = []
        }
    }
    $names | uniq
}

# fabro-7f58 shared runner: the full crate suite tier (existing tests
# edited, or narrowing fallback). Prints its own verdict; returns success.
def full-crate-suite [c: string] {
    print $"== nextest -p ($c) (full crate suite: existing tests edited) =="
    let res = (do { ^cargo nextest run -p $c --no-fail-fast } | complete)
    if $res.exit_code != 0 {
        print (output-tail ($res.stdout + $res.stderr) 3000)
        print $"verify: FAIL nextest ($c)"
        $env.LAST_VERIFY_FAIL = "nextest"
        return false
    }
    print $"verify: PASS nextest ($c)"
    true
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
        mut narrowed_ok = false
        if ($c not-in $t.edited_test_crates) {
            let names = ($t.added_test_files
                | where crate == $c
                | get path
                | each {|p| test-fn-names $p }
                | flatten
                | uniq)
            if ($names | is-not-empty) {
                print $"== nextest -p ($c) (new tests by name filter: ($names | length) names from purely-added test files) =="
                let res = (do { ^cargo nextest run -p $c --no-fail-fast ...$names } | complete)
                if $res.exit_code != 0 {
                    print (output-tail ($res.stdout + $res.stderr) 3000)
                    print $"verify: FAIL nextest ($c)"
                    $env.LAST_VERIFY_FAIL = "nextest"
                    return
                }
                # Fail-safe: a filter set that matches ZERO tests would exit
                # green without verifying anything — fall back to the full
                # crate suite instead of trusting a 0-test pass.
                let ran = ($res.stdout | parse --regex '(?P<n>\d+) tests run' | get -o n.0)
                if ($ran != null) and (($ran | into int) > 0) {
                    print $"verify: PASS nextest ($c) (narrowed: ($ran) tests)"
                    $narrowed_ok = true
                } else {
                    print "verify: name filter matched no tests — falling back to the full crate suite"
                }
            }
        }
        if (not $narrowed_ok) {
            if (not (full-crate-suite $c)) { return }
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
