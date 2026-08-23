Seed `fabro-8d26` implemented. Summary of the pass:

**Changes** (only `/workspace/fabro/main.go` and `/workspace/fabro/fib_test.go`; `go.mod` untouched, stdlib only):

- `main.go`: added package-level `const Version = "1.3.0"`; registered a `version` bool flag; extended the `run()` seam with a `version bool` parameter whose branch runs **before** the `count < 1` validation — this encodes the resolved spec reading that `-version` outranks all modes including count validation, and makes every combination unit-testable without process execution. Doc comments (package + `run`) updated to describe the new flag and its precedence.
- `fib_test.go`: threaded `false` through the 7 existing `run()` call sites; added table-driven `TestRunVersion` with 5 cases — `-version` alone, `-version -json -n 5`, `-version -pretty`, `-version -n 0`, `-version -json -n 0` — each asserting exactly one line `gofib 1.3.0` and the nil-error (exit-0) path.

**Smoke checks** (full gate left to the tester step): `gofmt -l` clean, `go vet` clean, `go test ./...` pass; `go run . -version` prints exactly `gofib 1.3.0` with exit 0, all precedence combos verified, and `gofib -n 0` without `-version` still fails as before. No build artifacts left in the worktree. Recorded the `run-seam-flag-precedence` pattern in mulch. No new workflow friction this pass.

Implemented:
{
  "outcome": "succeeded",
  "preferred_next_label": "Implemented",
  "context_updates": {
    "implementation_summary": "Added -version to gofib via the run() seam: package-level const Version = \"1.3.0\" and a version bool parameter on run() whose early-return branch precedes the count<1 validation, so -version outranks all output modes (verified for -pretty, -json -n 5, and -n 0). main.go registers the flag and passes it through; doc comments updated. fib_test.go threads false through all 7 existing run() call sites and adds table-driven TestRunVersion (5 cases incl. -version alone, -version -json -n 5, and both -n 0 precedence cases) asserting the single 'gofib 1.3.0' line and nil error. Smoke: gofmt/vet/go test green, go run . -version prints exactly 'gofib 1.3.0' exit 0. Files touched: /workspace/fabro/main.go, /workspace/fabro/fib_test.go. go.mod unchanged, no artifacts.",
    "workflow_painpoints": ["Seed fabro-d810 (priority 1) shows as ready but its remaining acceptance (recorded preamble-size numbers across >=3 cycles under the rebuilt denkhaus binary) is not executable from the lab sandbox: no fabro CLI, no run-event store in .fabro/, no meta world access; its workflow-edit portion is already synced into the worktree (commits 17e1fb8/965fbc8/e579afe) but the seed forbids closing without recorded numbers. Cross-world prerequisite (denkhaus fabro-29f7 rebuild) is not modeled as a blocker, so every planner pass re-trips over it and a naive claim would deadlock the implementer. Suggestion: either add a sd dep to fabro-d810 blocking on the denkhaus-side sequence, or re-scope the seed into a lab-side verification checklist whose inputs (run-event exports) are staged into the repo."]
  }
}