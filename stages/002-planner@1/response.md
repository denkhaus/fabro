Seed `fabro-a1f5` claimed. Brief for the Implementer:

**Seed:** `fabro-a1f5` — gofib: add -json flag for machine-readable output

**Brief:** In the Go module at `/workspace/fabro` (source in `/workspace/fabro/main.go`, tests in `/workspace/fabro/fib_test.go`), add a `-json` bool flag via the flag package so `gofib -json` and `gofib -json -n 5` work. JSON mode emits JSON Lines: one object per line, exactly `{"index": <int>, "fib": "<value-as-string>"}`, with big.Int fib values serialized as strings via `encoding/json` (stdlib only, no new deps). Text mode stays unchanged: `<index>: <value>`, default 100 numbers, `-n` controls count; flags combine so `-json -n 10` emits exactly 10 JSON lines. Invalid usage (`n < 1`) still exits non-zero with a stderr error in both modes — keep the existing pattern where `run` returns an error and `main` prints it and exits 1. This requires extending `run`'s signature (e.g., a `jsonMode bool` parameter) and updating existing tests. Extend `fib_test.go` with table-driven tests for `run` covering json+n=1, json default (100 lines), and combined json+n=10; assert exact output by unmarshalling each line and comparing fields. Acceptance: `go run . -json -n 5` emits exactly 5 valid JSON lines with correct fib values, and `just qualitygate` is green.

{
  "outcome": "succeeded",
  "preferred_next_label": "Seed claimed",
  "context_updates": {
    "current_seed_id": "fabro-a1f5",
    "current_seed_title": "gofib: add -json flag for machine-readable output",
    "current_seed_brief": "In the Go module at `/workspace/fabro` (`main.go`, `fib_test.go`), add a `-json` bool flag via the flag package. JSON mode emits JSON Lines: one object per line, exactly {\"index\": <int>, \"fib\": \"<value-as-string>\"}, with big.Int fib values serialized as strings using `encoding/json` (stdlib only). Text mode unchanged: `<index>: <value>`, default 100 numbers, `-n` controls count; flags combine (`-json -n 10` emits exactly 10 JSON lines). Invalid usage (n < 1) still exits non-zero with stderr error in both modes. Extend `run`'s signature (e.g., jsonMode bool) and update existing tests; extend `fib_test.go` with table-driven tests for `run` covering json+n=1, json default, and combined json+n=10, asserting exact lines by unmarshalling each and comparing fields. Acceptance: `go run . -json -n 5` emits exactly 5 valid JSON lines with correct fib values and `just qualitygate` is green."
  }
}