Seed `fabro-f487` claimed and set to `in_progress`.

**Implementation brief — fabro-f487: gofib: Go CLI printing the first 100 Fibonacci numbers**

Build a small Go CLI named `gofib` at the repository root. Requirements:
- Go module `gofib` at repo root (`go.mod`, Go 1.27), stdlib only — no external dependencies.
- `package main` in `main.go`; the `gofib` command prints the first 100 Fibonacci numbers, one per line, prefixed with index: `1: 1`, `2: 1`, `3: 2`, … `100: 354224848179261915075`.
- F(100) overflows int64 — use `math/big`.
- Fibonacci computation lives in a separate unit-testable function `Fib(n int) *big.Int`.
- Add `fib_test.go` with table-driven tests covering F(1), F(2), F(10), F(100).
- Acceptance: `go run .` prints exactly 100 correct lines, and `just qualitygate` passes (gofmt clean, `go vet`, `go build ./...`, `go test ./...`).

{
  "outcome": "succeeded",
  "preferred_next_label": "Seed claimed",
  "context_updates": {
    "current_seed_id": "fabro-f487",
    "current_seed_title": "gofib: Go CLI printing the first 100 Fibonacci numbers",
    "current_seed_brief": "Build a Go CLI 'gofib' at the repo root: Go module 'gofib' (go 1.27, stdlib only), package main in main.go printing the first 100 Fibonacci numbers one per line prefixed with index (e.g. '1: 1' ... '100: 354224848179261915075'). Use math/big since F(100) exceeds int64; expose the computation as a testable Fib(n int) *big.Int function. Include fib_test.go with table-driven tests for F(1), F(2), F(10), F(100). Done when 'go run .' prints exactly 100 correct lines and 'just qualitygate' (gofmt, go vet, go build ./..., go test ./...) is green."
  }
}