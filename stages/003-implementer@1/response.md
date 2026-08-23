Implementation complete. Summary of this pass:

**`/workspace/fabro/main.go`**
- Added package-level const `Version = "1.3.0"`.
- `run()` gained a `version bool` parameter; when set it prints exactly `gofib <Version>\n` and returns nil. The check sits **before** the `count < 1` validation, so `gofib -version -n 0` still prints the version line and exits 0, per the brief's ambiguity resolution.
- `main()` registers the `-version` bool flag via the flag package and threads it into `run()`.
- Updated the package and `run()` doc comments.

**`/workspace/fabro/fib_test.go`**
- All 7 existing `run()` call sites updated for the new parameter.
- New table-driven `TestRunVersion` covering: `-version` alone, `-version -json -n 5`, `-version -pretty`, and `-version -n 0` (precedence over validation) — each asserts the single line `gofib 1.3.0` and nil error.

**Smoke checks (gate left to the tester step):** `gofmt` clean, `go vet` clean, `go test ./...` passes; `go run . -version`, `-version -pretty`, `-version -json -n 5`, and `-version -n 0` all print only `gofib 1.3.0` with exit 0; normal modes unaffected; no binary left in the worktree (used `go run .`, which builds into the Go cache); `git status` shows only the two intended files modified. No new mulch insight surfaced — the applied thin-main pattern was already recorded (`mx-d4cf38`); no painpoints.

Implemented:
{
  "outcome": "succeeded",
  "preferred_next_label": "Implemented",
  "context_updates": {
    "implementation_summary": "In `/workspace/fabro/main.go`: added package const `Version = \"1.3.0\"`, threaded a `version bool` into `run()` whose check prints `gofib <Version>` and returns nil before the `-n` validation (so `-version -n 0` wins), registered the `-version` flag in `main()`, and updated doc comments. In `/workspace/fabro/fib_test.go`: updated all 7 existing `run()` call sites and added table-driven `TestRunVersion` (version alone, with `-json -n 5`, with `-pretty`, with invalid `-n 0`; each asserts the single `gofib 1.3.0` line and nil error). Smoke checks green: gofmt/vet/test pass, all four `-version` invocation shapes print only the version line with exit 0, no artifacts left in the worktree."
  }
}