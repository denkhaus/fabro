Seed `fabro-759d` claimed. Brief for the Implementer:

- **Seed**: `fabro-759d` — gofib: add `-n` flag to control how many Fibonacci numbers are printed
- **Build**: Extend the existing `main.go` gofib CLI with a `-n` int flag via the stdlib `flag` package. Default 100 keeps current behavior (`gofib` prints the first 100 Fibonacci numbers as `<index>: <value>`, one per line, using `math/big`). `gofib -n 10` prints exactly the first 10 in the same format. Usage text for the flag must explain the default. Validate `n >= 1`; values of 0 or negative must exit non-zero with an error on stderr (prefer extracting the run logic into a testable function, e.g. `run(args, stdout, stderr) int`, over os/exec).
- **Tests**: Extend `fib_test.go` with table-driven tests covering n=1, n=10, and the default path, plus one error-case test for n=0 asserting non-zero exit and a stderr message.
- **Constraints**: No external dependencies. Existing `Fib` behavior and `TestFib` must stay green.
- **Done when**: `go run . -n 5` prints exactly 5 correct lines and `just qualitygate` passes.

Claimed a seed:
{
  "outcome": "succeeded",
  "preferred_next_label": "Seed claimed",
  "context_updates": {
    "current_seed_id": "fabro-759d",
    "current_seed_title": "gofib: add -n flag to control how many Fibonacci numbers are printed",
    "current_seed_brief": "Extend the existing gofib CLI (main.go) with a -n int flag using the stdlib flag package. Default 100 preserves current behavior: '<index>: <value>' one per line via math/big. 'gofib -n 10' prints exactly the first 10 in the same format; usage text must explain the default. Validate n >= 1 — 0 or negative exits non-zero with an error message on stderr (extract run logic into a testable function, e.g. run(args, stdout, stderr) int, rather than os/exec). Extend fib_test.go: table-driven tests for n=1, n=10, and the default path, plus one error test for n=0 asserting non-zero exit and stderr output. No external dependencies; existing Fib/TestFib must stay green. Done when 'go run . -n 5' prints exactly 5 correct lines and 'just qualitygate' passes."
  }
}