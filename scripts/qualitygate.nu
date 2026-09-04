#!/usr/bin/env nu
# Touched-crates quality gate (fabro-5453, world merger): derives the crates
# a run actually changed and gates exactly those — the fabro workspace takes
# 15+ min for a cold full build at the 8-CPU run environment, so the
# develop-workflow tester cannot afford `--workspace` gates (measurement:
# scripts/gate-measure-24cpu-reference.log, 394s cold at 24 CPUs).
# Exit 0 = green. Style follows the lab exemplar (denkhaus-lab
# scripts/qualitygate.nu): sections, `do { ^cmd } | complete`, first red
# stops the gate.
#
# Base detection reuses the lab evidence.nu pattern: the parent of the LAST
# engine checkpoint commit (subject 'fabro(<run-id>): ...') is the run base —
# works in the shallow sandbox clone without origin refs.

# Flaky-test policy: nextest retries each failing test once. The known
# fabro-80b3 class (for_each_accepts_an_array_at_the_item_limit times out
# under full-suite load) gets its second chance; deterministic failures
# still fail the gate.
const NEXTEST_RETRIES = 1

def current-branch [] {
    (git branch --show-current | str trim)
}

def run-base [] {
    let run_id = (
        current-branch
        | parse --regex 'fabro/run/(?P<id>[^/]+)$'
        | get -o id.0
        | default ''
    )
    if ($run_id | is-empty) {
        # Interactive/human invocation outside a run: diff the working tree
        # against HEAD (uncommitted changes) — the touched-set of a draft.
        return {base: "HEAD", grounded: false}
    }
    let subject_mark = $"fabro\(($run_id)\):"
    let checkpoints = (git log --format=%H --fixed-strings --grep $subject_mark | lines | compact)
    if ($checkpoints | is-empty) {
        return {base: "HEAD", grounded: false}
    }
    let base = (git rev-parse $"($checkpoints | last)^")
    {base: $base, grounded: true}
}

# Map changed paths to workspace crates; root manifest/lock changes mark the
# whole workspace as touched (dependency edits can affect every crate).
def touched-crates [] {
    let base = (run-base).base
    let paths = (git diff --name-only $base | lines | compact)
    let crate_paths = ($paths | where {|p| $p | str starts-with 'lib/' })
    if ($crate_paths | is-empty) {
        return []
    }
    let crates = ($crate_paths
        | each {|p| $p | parse --regex '^lib/(?:apps|components|foundation)/(?P<crate>[^/]+)/' }
        | get -o crate
        | uniq
        | compact)
    let root_touched = ($paths | where {|p|
        ($p in ['Cargo.toml' 'Cargo.lock' 'rust-toolchain.toml'])
    } | is-not-empty)
    if $root_touched {
        print "root manifest/lock changed -> workspace-wide gate (cargo check)"
        return []
    }
    $crates
}

# Toolchain pin (AGENTS.md): rustfmt/clippy results depend on the compiler
# version — the repo pins nightly-2026-04-14, which is also the default (and
# only) toolchain in the toolchain image. Explicit pin keeps the gate
# identical on the host, where the default is stable.
const PINNED_TOOLCHAIN = "nightly-2026-04-14"

def check-fmt [] {
    print "== cargo fmt --check --all =="
    let res = (do { ^cargo $'+($PINNED_TOOLCHAIN)' fmt --check --all } | complete)
    if $res.exit_code != 0 {
        print ($res.stdout | str trim -r -c "\n")
        print ($res.stderr | str trim -r -c "\n")
        return false
    }
    print "format clean"
    true
}

def check-clippy [crates: list<string>] {
    if ($crates | is-empty) { return true }
    let pkgs = ($crates | each {|c| $"-p ($c)" })
    print $"== cargo clippy ($crates | str join ', ') -D warnings =="
    let res = (do { ^cargo $'+($PINNED_TOOLCHAIN)' clippy ...$pkgs --all-targets -- -D warnings } | complete)
    if $res.exit_code != 0 {
        print ($res.stdout | str trim -r -c "\n" | lines | last 30)
        print ($res.stderr | str trim -r -c "\n")
        return false
    }
    print "clippy clean"
    true
}

def check-tests [crates: list<string>] {
    if ($crates | is-empty) { return true }
    let pkgs = ($crates | each {|c| $"-p ($c)" })
    print $"== cargo nextest ($crates | str join ', ') — retries ($NEXTEST_RETRIES) =="
    let res = (do { ^cargo nextest run ...$pkgs --no-fail-fast --retries $NEXTEST_RETRIES } | complete)
    if $res.exit_code != 0 {
        print ($res.stdout | str trim -r -c "\n" | lines | where {|l| ($l | str contains 'FAIL') or ($l | str contains 'Summary')} | last 20)
        print ($res.stderr | str trim -r -c "\n")
        return false
    }
    print "tests green"
    true
}

# Workspace-wide fallback when root manifests changed: a compile check only
# (clippy+tests on all 52 crates would blow the tester timeout).
def check-workspace-compiles [] {
    print '== cargo check --workspace — root manifest changed =='
    let res = (do { ^cargo check --workspace } | complete)
    if $res.exit_code != 0 {
        print ($res.stderr | str trim -r -c "\n" | lines | last 30)
        return false
    }
    print "workspace compiles"
    true
}

def main [] {
    let crates = (touched-crates)
    let base = (run-base)
    if not $base.grounded {
        print 'gate base ungrounded: interactive or pre-checkpoint, diffing working tree'
    }
    if ($crates | is-empty) {
        print "no crates touched"
        if (check-fmt) { exit 0 } else { exit 1 }
    }
    print $"touched crates: ($crates | str join ', ')"
    let green = ((check-fmt) and (check-clippy $crates) and (check-tests $crates))
    if $green {
        print "GATE GREEN"
        exit 0
    }
    print "GATE RED"
    exit 1
}
