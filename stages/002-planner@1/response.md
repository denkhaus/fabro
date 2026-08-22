Not yet implemented — this is a fresh implementation cycle. The spec is internally consistent (bool flag, const `Version = "1.3.0"`, precedence over all output modes, table-driven test for `-version` alone and with `-json`, no deps, `just qualitygate` green).

{
  "outcome": "succeeded",
  "preferred_next_label": "Seed claimed",
  "context_updates": {
    "current_seed_id": "fabro-8d26",
    "current_seed_title": "gofib: add -version flag",
    "current_seed_brief": "Add a -version flag to the gofib CLI in `main.go` (repo root `/workspace/fabro`). Acceptance criteria: - define a package-level const `Version = \"1.3.0\"`; - add a bool flag `version` via the flag package so `gofib -version` works; - when -version is set, print exactly one line to stdout: `gofib <semver>` (i.e. `gofib 1.3.0`) and exit 0; - -version takes precedence over every output mode: `gofib -version -pretty` and `gofib -version -json -n 5` print only the version line (no fib output, no JSON); - add a table-driven test (in `fib_test.go` or a new `main_test.go`) covering run()/main-level behavior for -version alone and -version combined with -json, asserting the single version line and exit code 0; - no external dependencies; - `just qualitygate` must pass; - done when `go run . -version` prints `gofib 1.3.0`. No spec contradictions found."
  }
}