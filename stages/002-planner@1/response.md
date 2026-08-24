Claimed `fabro-cfb6` (gofib: `-seed` flag). No review verdict to handle; guards clean (fresh cycle, counters at 0). The spec aligns with existing flag patterns in `main.go` and the README table; one ambiguity (flag name suggests randomness but semantics are a single-index lookup; also `-seed 0` sentinel behavior) is annotated in the brief below.

- `-seed <int>` flag: prints exactly ONE entry for index `i` (one line) in the active output mode — text: `i: <value>`; `-json`: one object `{"index":i,"fib":"<string>"}`; `-pretty`: single-row alignment (columns sized to that one row)
- `-seed i` (i > 0) overrides the range flags: `-n`, `-start`, and `-limit` are ignored for output — e.g. `-seed 10 -n 5 -start 2 -limit 3` prints only index 10
- `-version` keeps top precedence: `-version -seed 5` prints the version and exits 0
- `-seed 0` is a sentinel treated as UNSET (mirrors `-start 0`), so `gofib -seed 0 -n 5` behaves exactly like `gofib -n 5` — plain output unchanged
- Validation mirrors other flags: `-seed -1` exits non-zero with a stderr error (`invalid value -1 for flag -seed: must be >= 0`); chosen reading: `-seed` overrides which indices print, not other flags' validation — existing `-n`/`-start`/`-limit` validation behavior stays unchanged
- Computation must go through the existing `Fib(n)` only — no inline recomputation (CONTEXT.md rule)
- Tests: table-driven in `fib_test.go` — cover: single index via `-seed`; `-seed` overriding `-n`/`-start`/`-limit`; `-seed 0` sentinel; `-seed` with `-json`; `-seed` with `-pretty`; negative `-seed` rejected non-zero; `-version` still wins over `-seed`
- README.md flag table gains a `-seed` row: real example invocation + its actual output captured from the built binary (do not invent output); the precedence-over-range-flags note lives in that row
- Gate: `just qualitygate` green

{
  "outcome": "succeeded",
  "preferred_next_label": "Seed claimed",
  "context_updates": {
    "current_seed_id": "fabro-cfb6",
    "current_seed_title": "gofib: add -seed flag for reproducible index selection",
    "current_seed_brief": "- `-seed <int>` flag: prints exactly ONE entry for index `i` (one line) in the active output mode — text: `i: <value>`; `-json`: one object `{\"index\":i,\"fib\":\"<string>\"}`; `-pretty`: single-row alignment (columns sized to that one row)\n- `-seed i` (i > 0) overrides the range flags: `-n`, `-start`, and `-limit` are ignored for output — e.g. `-seed 10 -n 5 -start 2 -limit 3` prints only index 10\n- `-version` keeps top precedence: `-version -seed 5` prints the version and exits 0\n- `-seed 0` is a sentinel treated as UNSET (mirrors `-start 0`), so `gofib -seed 0 -n 5` behaves exactly like `gofib -n 5` — plain output unchanged\n- Validation mirrors other flags: `-seed -1` exits non-zero with a stderr error (`invalid value -1 for flag -seed: must be >= 0`); chosen reading: `-seed` overrides which indices print, not other flags' validation — existing `-n`/`-start`/`-limit` validation behavior stays unchanged\n- Computation must go through the existing `Fib(n)` only — no inline recomputation (CONTEXT.md rule)\n- Tests: table-driven in `fib_test.go` — cover: single index via `-seed`; `-seed` overriding `-n`/`-start`/`-limit`; `-seed 0` sentinel; `-seed` with `-json`; `-seed` with `-pretty`; negative `-seed` rejected non-zero; `-version` still wins over `-seed`\n- README.md flag table gains a `-seed` row: real example invocation + its actual output captured from the built binary (do not invent output); the precedence-over-range-flags note lives in that row\n- Gate: `just qualitygate` green"
  }
}