Claimed `fabro-8d26`. Spec is internally consistent; one ambiguity annotated in the brief below (version precedence vs. `-n` validation order).

{
  "outcome": "succeeded",
  "preferred_next_label": "Seed claimed",
  "context_updates": {
    "current_seed_id": "fabro-8d26",
    "current_seed_title": "gofib: add -version flag",
    "current_seed_brief": "Add a -version flag to the gofib CLI in `/workspace/fabro/main.go` (and tests in `/workspace/fabro/fib_test.go`). Acceptance criteria:\n- New bool flag `version` registered via the flag package in main(): usable as `gofib -version`.\n- Package-level const `Version = \"1.3.0\"` in main.go.\n- With -version, print exactly one line to stdout: `gofib 1.3.0` (i.e. `gofib <Version>`), then exit 0.\n- -version takes precedence over all output modes: `gofib -version -pretty` and `gofib -version -json -n 5` print only the version line. Ambiguity resolved: check -version BEFORE any -n validation (current code rejects -n < 1 in run() before printing), so the version check must happen ahead of that error path — `gofib -version -n 0` still prints the version line and exits 0.\n- Add a table-driven test (extend `fib_test.go`) covering: -version alone, and -version combined with -json; each case asserts the single line `gofib 1.3.0` on stdout and success (no error / exit 0). Test run()-level or main-level behavior — since main() is untested today, the natural shape is threading the version flag into run() or an equivalent testable seam; keep it minimal.\n- No external dependencies; `just qualitygate` in `/workspace/fabro` must pass.\nDone when `go run . -version` prints `gofib 1.3.0` and the gate is green."
  }
}