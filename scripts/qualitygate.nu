#!/usr/bin/env nu
# Deterministic quality gate. Exit 0 = green, non-zero = red.
# Called by `just qualitygate`; the develop workflow's tester step goes
# through the same target, so there is exactly one gate definition.

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
go build ./...

print "== go test =="
go test ./...

print "== qualitygate passed =="
