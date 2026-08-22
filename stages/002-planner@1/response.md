Claimed `fabro-f487`. Brief for the Implementer:

**Seed fabro-f487 — gofib: Go CLI printing the first 100 Fibonacci numbers**

Build a Go CLI named `gofib` at the repository root: a `go.mod` declaring module `gofib` (go 1.27), `main.go` in package main, and `fib_test.go`. The command prints the first 100 Fibonacci numbers, one per line, as `<index>: <value>` (e.g. `1: 1`, `2: 1`, `3: 2`, … `100: 354224848179261915075`). Since F(100) overflows int64, the computation must use `math/big`, isolated in a unit-testable function `Fib(n int) *big.Int` separate from `main`. Tests are table-driven and cover F(1), F(2), F(10), F(100). Stdlib only, no external dependencies.

Acceptance criteria:
- `go run .` prints exactly 100 lines, correctly formatted and valued.
- `Fib` is a standalone function with table-driven tests for F(1), F(2), F(10), F(100).
- `just qualitygate` passes: gofmt clean, `go vet`, `go build ./...`, `go test ./...`.

{
  "outcome": "succeeded",
  "preferred_next_label": "Seed claimed",
  "context_updates": {
    "current_seed_id": "fabro-f487",
    "current_seed_title": "gofib: Go CLI printing the first 100 Fibonacci numbers",
    "current_seed_brief": "Build a Go CLI 'gofib' at the repo root: go.mod (module gofib, go 1.27), main.go in package main, and fib_test.go. 'gofib' prints the first 100 Fibonacci numbers, one per line, as '<index>: <value>' ending at '100: 354224848179261915075'. F(100) exceeds int64 so use math/big, with the computation in a standalone unit-testable function Fib(n int) *big.Int separate from main. Table-driven tests must cover F(1), F(2), F(10), F(100). Stdlib only, no external deps. Acceptance: 'go run .' prints exactly 100 correct lines and 'just qualitygate' passes (gofmt clean, go vet, go build ./..., go test ./...)."
  }
}