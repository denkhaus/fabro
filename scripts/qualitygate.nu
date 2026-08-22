#!/usr/bin/env nu
# Deterministic quality gate. Exit 0 = green, non-zero = red.
# Called by `just qualitygate`; the develop workflow's tester step goes
# through the same target, so there is exactly one gate definition.

print "== workflow sync (product <-> meta) =="
nu .fabro/workflows/develop/scripts/sync-check.nu

print "== tracked large files =="
let big = (
    git ls-files
    | lines
    | par-each {|f| {file: $f, size: (ls $f | get 0.size)}}
    | where size > 1mb
)
if ($big | length) > 0 {
    print "build artifacts or large binaries are tracked — untrack them (.gitignore + git rm --cached):"
    $big | each {|it| print $"($it.size) \t ($it.file)"}
    exit 1
}

print "== gofmt check =="
let unformatted = (gofmt -l . | lines | compact)
if ($unformatted | length) > 0 {
    print "unformatted files:"
    print ($unformatted | str join "\n")
    exit 1
}

print "== go vet =="
go vet ./...

print "== go build =="
# Build into a temp dir OUTSIDE the worktree: a bare `go build ./...` drops
# the `gofib` binary into the repo root on every gate run (artifact hygiene).
let out = (mktemp -d)
go build -o $"($out)/" ./...
rm -rf $out

print "== go test =="
go test ./...

print "== qualitygate passed =="
