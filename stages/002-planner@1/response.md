Seed claimed. Brief distilled from the seed description and verified against the worktree (CLI exists in `/workspace/fabro/main.go`, no JSON support yet — fresh implementation work).

**Brief for fabro-a1f5 — gofib: add -json flag for machine-readable output:**
Extend the gofib CLI at `/workspace/fabro/main.go` with a `-json` bool flag (flag package). In JSON mode, emit JSON Lines — one object per line, exactly `{"index": <int>, "fib": "<value-as-string>"}` — with big-number fib values serialized as strings; default text mode (`<index>: <value>`) stays unchanged, and flags combine (`-json -n 10` emits exactly 10 JSON lines). Invalid usage (`n < 1`) must exit non-zero with a stderr error in both modes. Extend `/workspace/fabro/fib_test.go` with table-driven tests for `run()` covering json+n=1, json default, and combined `-json -n 10`, asserting exact JSON lines by unmarshalling each line and comparing fields. Stdlib only (`encoding/json` allowed); no new dependencies. Done when `go run . -json -n 5` emits exactly 5 valid JSON lines with correct fib values and `just qualitygate` passes.

{
  "outcome": "succeeded",
  "preferred_next_label": "Seed claimed",
  "context_updates": {
    "current_seed_id": "fabro-a1f5",
    "current_seed_title": "gofib: add -json flag for machine-readable output",
    "current_seed_brief": "Extend the gofib CLI in `/workspace/fabro/main.go` with a bool flag `-json` (flag package). JSON mode emits JSON Lines: one object per line, exactly {\"index\": <int>, \"fib\": \"<value-as-string>\"}, with big fib values serialized as strings; default text mode (<index>: <value>) unchanged. Flags combine: -json -n 10 emits exactly 10 JSON lines. Invalid usage (n < 1) exits non-zero with a stderr error in both modes. Extend `/workspace/fabro/fib_test.go` with table-driven tests for run() covering json+n=1, json default, and combined -json -n 10, asserting exact JSON lines by unmarshalling each line and comparing fields. Stdlib only (encoding/json allowed). Done when `go run . -json -n 5` emits exactly 5 valid JSON lines with correct fib values and `just qualitygate` is green."
  }
}