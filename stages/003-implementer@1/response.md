Implementation of seed `fabro-a1f5` is complete. Summary:

**Changes to `main.go`:**
- Added `fibLine` struct (`{"index": int, "fib": string}`) — fib serialized as string since values exceed int64.
- `run` gained an `asJSON bool` parameter; JSON mode uses a `json.Encoder` to emit one object per line (JSON Lines), text mode unchanged. Count validation happens before any output, so invalid `-n` errors identically in both modes.
- `main` wires up the new `-json` bool flag via the flag package, combinable with `-n`.

**Changes to `fib_test.go`:**
- Updated existing `TestRun` call sites for the new signature.
- Added `TestRunJSON`: table-driven for json+n=1, json default (100 lines), and combined json+n=10; unmarshals every line, compares `index`/`fib` fields, and pins the exact canonical line shape.
- `TestRunRejectsInvalidCount` now loops over both text and JSON modes.

**Smoke checks (not the full gate):** gofmt clean, `go vet`, `go build`, `go test` all pass; `go run . -json -n 5` emits exactly 5 valid JSON lines and `go run . -json -n 0` exits 1 with the stderr error. Only source files modified — no artifacts in the worktree.

Implemented:
{
  "outcome": "succeeded",
  "preferred_next_label": "Implemented",
  "context_updates": {
    "implementation_summary": "Extended gofib with a -json flag per seed fabro-a1f5: main.go adds a fibLine struct and an asJSON parameter to run(), emitting JSON Lines via json.Encoder while text mode stays unchanged, with -n validation before any output in both modes; fib_test.go adds TestRunJSON (json+n=1, json default, json+n=10, unmarshal-and-compare plus exact-line check) and extends the invalid-count test to both modes. gofmt/vet/build/test pass and `go run . -json -n 5` emits 5 valid JSON lines; no artifacts left behind."
  }
}