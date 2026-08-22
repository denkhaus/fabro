Implementation complete and verified. Summary against the seed's acceptance criteria:

- **`Version` const**: package-level `const Version = "1.3.0"` in `main.go` with doc comment.
- **Bool `version` flag**: via `flag` package; `gofib -version` prints exactly `gofib 1.3.0` (one line, stdout, exit 0) — verified by `go run . -version`.
- **Precedence**: version check early-returns in `runApp` before any output mode; smoke-verified `gofib -version -pretty` and `gofib -version -json -n 5` print only the version line.
- **Table-driven test**: new `main_test.go` with `TestRunAppVersion` (cases: `-version` alone, `-version -json -n 5`, `-version -pretty`) asserting the exact single line and exit code 0, plus `TestRunAppDispatch` guarding that normal modes (`-n 3` text, `-json -n 2`) still dispatch correctly and `-n 0` exits 1.
- **Testability**: to make exit-code assertions possible at main level, `main()` is now a thin `os.Exit(runApp(os.Stdout, os.Args[1:]))` using a per-call `flag.NewFlagSet` (behavior-preserving: `-h`→0, flag error→2, `run` error→1); `run()` and all existing tests untouched.
- **No external deps**; smoke checks green: `gofmt` clean, `go vet` clean, `go test ./...` ok. Worktree contains only the source changes — no binaries or generated artifacts. Full `just qualitygate` left to the tester step.

Recorded the testable-main pattern in mulch (`ml record gofib --type pattern --name "runapp-testable-main"`).

Implemented:
{
  "outcome": "succeeded",
  "preferred_next_label": "Implemented",
  "context_updates": {
    "implementation_summary": "Modified `/workspace/fabro/main.go`: added const `Version = \"1.3.0\"`, restructured main into a testable `runApp(w, args) int` (per-call FlagSet, ErrHelp->0, parse error->2, run error->1) with a `version` bool flag that early-returns before all output modes, printing exactly `gofib 1.3.0`. Added `/workspace/fabro/main_test.go`: table-driven `TestRunAppVersion` (-version alone, with -json -n 5, with -pretty; asserts single line + exit 0) and `TestRunAppDispatch` passthrough guards. Smoke checks pass (gofmt, vet, go test, `go run . -version` -> `gofib 1.3.0`); no artifacts left."
  }
}