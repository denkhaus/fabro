Seed claimed. Brief distilled from the seed description and verified against the worktree (`/workspace/fabro/main.go` currently supports only `-n` and `-json`; no `-pretty` anywhere — this is fresh implementation work, not verification-only).

**Brief for fabro-0879 — gofib: add `-pretty` flag for aligned column output**

Extend the gofib CLI in `/workspace/fabro/main.go` with a `-pretty` output mode, plus table-driven tests in `/workspace/fabro/fib_test.go`. Requirements:

1. New bool flag `pretty` via the flag package — `gofib -pretty`, `gofib -pretty -n 10`, and `gofib -pretty -json` are all valid combinations.
2. Pretty TEXT mode: two aligned columns, both right-aligned, separated by `': '`:
   - `'   1:                    1'`, `'   2:                    1'`, `'  10:                   55'`, `' 100:  354224848179261915075'`
   - Index column width = width of the largest index; value column width = width of the largest value. Widths derive from `n` (for `-n 5` they come from the 5th line, not the 100th).
3. Pretty JSON mode: `-pretty` has no effect on JSON output — identical JSON Lines objects as `-json` (`{"index":<int>,"fib":"<string>"}`). Document this in the flag usage text.
4. `-pretty` and `-n` combine: pretty text for exactly `n` numbers.
5. Default output (no `-pretty`) is unchanged: `'<index>: <value>'` with single spaces.
6. Tests in `/workspace/fabro/fib_test.go`: table-driven tests for `run()` covering pretty+n=5 (exact expected 5 lines including padding), pretty default (assert line count 100 and the exact last line), pretty+json (identical lines to `-json` for n=3). Compute expected strings in tests — no hardcoded Fibonacci values beyond small `n`.
7. No external dependencies. `just qualitygate` must pass.

Done when `go run . -pretty -n 5` prints the exact aligned 5-line block (widths from the 5th line) and the gate is green.

No review feedback — this is the first pass on this seed. No planner painpoints to report.

{
  "outcome": "succeeded",
  "preferred_next_label": "Seed claimed",
  "context_updates": {
    "current_seed_id": "fabro-0879",
    "current_seed_title": "gofib: add -pretty flag for aligned column output",
    "current_seed_brief": "Extend the gofib CLI in `/workspace/fabro/main.go` with a bool -pretty flag and matching tests in `/workspace/fabro/fib_test.go`. (1) 'gofib -pretty', 'gofib -pretty -n 10', 'gofib -pretty -json' must all be valid. (2) Pretty TEXT mode: two right-aligned columns separated by ': '; index column width = width of largest index, value column width = width of largest value, both derived from n (for -n 5 the widths come from the 5th line). (3) -pretty does not change JSON mode: output stays identical JSON Lines ({\"index\":<int>,\"fib\":\"<string>\"}); document this in the flag usage text. (4) -pretty and -n combine. (5) Default output unchanged: '<index>: <value>' with single spaces. (6) Table-driven tests for run(): pretty+n=5 exact 5 lines including padding; pretty default asserts 100 lines plus the exact last line; pretty+json identical to -json for n=3; compute expected strings in tests, no hardcoded Fibonacci values beyond small n. (7) No external dependencies; 'just qualitygate' must pass. Done when 'go run . -pretty -n 5' prints the exact aligned 5-line block and the gate is green. No review feedback — first pass."
  }
}