#!/usr/bin/env nu
# Deterministic quality gate. Exit 0 = green, non-zero = red.
# Called by `just qualitygate`; the develop workflow's tester step goes
# through the same target, so there is exactly one gate definition.
# Each check prints its own section and returns false (with details) on
# failure; main stops at the first red check.

const LARGE_FILE_LIMIT = 1mb

# ---------------------------------------------------------------------------
# checks — nothing -> bool (false = red, details already printed)
# ---------------------------------------------------------------------------

def check-sync []: nothing -> bool {
    print "== workflow sync (product <-> meta) =="
    let res = (do { nu .fabro/workflows/develop/scripts/sync-check.nu } | complete)
    print ($res.stdout | str trim -r -c "\n")
    if $res.exit_code != 0 {
        print ($res.stderr | str trim -r -c "\n")
        return false
    }
    true
}

def check-large-files []: nothing -> bool {
    print "== tracked large files =="
    let big = (
        git ls-files
        | lines
        | par-each {|f| {file: $f, size: (ls $f | get 0.size)}}
        | where size > $LARGE_FILE_LIMIT
    )
    if ($big | length) > 0 {
        print "build artifacts or large binaries are tracked — untrack them (.gitignore + git rm --cached):"
        $big | each {|it| print $"($it.size) \t ($it.file)"}
        return false
    }
    true
}

def check-gofmt []: nothing -> bool {
    print "== gofmt check =="
    let unformatted = (gofmt -l . | lines | compact)
    if ($unformatted | length) > 0 {
        print "unformatted files:"
        print ($unformatted | str join "\n")
        return false
    }
    true
}

def check-go [phase: string]: nothing -> bool {
    print $"== go ($phase) =="
    let res = (do { ^go $phase ./... } | complete)
    if $res.exit_code != 0 {
        print $res.stdout
        print $res.stderr
        return false
    }
    print ($res.stdout | str trim -r -c "\n")
    true
}

# Build into a temp dir OUTSIDE the worktree: a bare `go build ./...` drops
# the compiled binary into the repo root on every gate run (artifact hygiene).
def check-go-build []: nothing -> bool {
    print "== go build =="
    let out = (mktemp --directory)
    let res = (do { go build -o $"($out)/" ./... } | complete)
    rm -rf $out
    if $res.exit_code != 0 {
        print $res.stdout
        print $res.stderr
        return false
    }
    true
}

# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------

def main []: nothing -> nothing {
    if not (check-sync) { exit 1 }
    if not (check-large-files) { exit 1 }
    if not (check-gofmt) { exit 1 }
    if not (check-go-build) { exit 1 }
    if not (check-go vet) { exit 1 }
    if not (check-go test) { exit 1 }
    print "== qualitygate passed =="
}
