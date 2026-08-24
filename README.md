# gofib

gofib is a Go CLI printing Fibonacci numbers. By default it prints the
first 100, one per line, prefixed with the index. Module `gofib`
(package `main` in `main.go`).

## Install / build

The workspace toolchain (Go, just, nushell, bun) is managed with
[mise](https://mise.jdx.dev/), and project commands are thin `just`
recipes:

```
mise install      # install the pinned toolchain (.mise.toml)
just bootstrap    # bootstrap workspace tooling
```

Build and run from the repo root:

```
go build -o /tmp/gofib .
/tmp/gofib -n 5
```

The deterministic quality gate (format, vet, build, test) is:

```
just qualitygate
```

## Usage

Text mode (the default) prints one `<index>: <value>` line per number:

```
$ gofib -n 5
1: 1
2: 1
3: 2
4: 3
5: 5
```

With no count flag, gofib prints the first 100 — F(100) already exceeds
int64, which is why values are computed with `math/big`:

```
$ gofib | tail -1
100: 354224848179261915075
```

`-start` changes the first index printed without changing what `-n`
counts — `-start s -n k` prints exactly `k` lines, indices `s..s+k-1`.
The default (`0`) behaves like `1`, so plain `gofib` output is
unchanged:

```
$ gofib -start 10 -n 5
10: 55
11: 89
12: 144
13: 233
14: 377
```

`-limit` caps the largest index printed, independent of `-start` and
`-n` (neither changes what the other counts). The output is the
intersection of all three: `start <= index <= min(start+n-1, limit)`,
at most `-n` lines. The default (`0`) means no limit, so unset output
is unchanged — and a huge `-n` can no longer flood the terminal:

```
$ gofib -n 100000 -limit 5
1: 1
2: 1
3: 2
4: 3
5: 5

$ gofib -start 10 -n 5 -limit 12
10: 55
11: 89
12: 144
```

`-limit` smaller than `-start` (both positive) is **not an error**:
gofib prints nothing and exits `0` — the ranges simply do not
intersect:

```
$ gofib -start 10 -n 5 -limit 7
$ echo $?
0
```

JSON mode (`-json`) emits JSON Lines — one object per number, never a
JSON array. `fib` is a string (ADR-0001) because F(100) overflows
int64:

```
$ gofib -n 3 -json
{"index":1,"fib":"1"}
{"index":2,"fib":"1"}
{"index":3,"fib":"2"}

$ gofib -json | tail -1
{"index":100,"fib":"354224848179261915075"}
```

`-pretty` aligns text-mode output into two right-aligned columns sized
to the largest index and value printed:

```
$ gofib -n 12 -pretty
 1:   1
 2:   1
 3:   2
 4:   3
 5:   5
 6:   8
 7:  13
 8:  21
 9:  34
10:  55
11:  89
12: 144
```

`-version` prints the version and exits:

```
$ gofib -version
gofib 1.3.0
```

An invalid count fails with a non-zero exit and an error on stderr:

```
$ gofib -n 0
invalid value 0 for flag -n: must be >= 1
$ echo $?
1
```

## Flag reference

| Flag | Meaning |
|------|---------|
| `-n <int>` | Count flag: how many numbers to print. Default `100`; must be `>= 1`, otherwise gofib exits non-zero with an error on stderr. Combines with JSON mode. |
| `-start <int>` | Start flag: index of the first number printed. Default `0`, which behaves like `1` (gofib's indices are 1-based), so unset output is unchanged — plain `gofib` still starts `1: 1` and ends `100: 354224848179261915075`. `-start s -n k` prints exactly `k` lines with indices `s..s+k-1`: `-start` never changes what `-n` counts. Must be `>= 0`, otherwise gofib exits non-zero with an error on stderr. Combines with `-json` (the `index` field carries the actual index) and `-pretty` (columns sized to the largest index and value actually printed). |
| `-limit <int>` | Cap flag: the largest index gofib will print, independent of `-start` and `-n`. Default `0`, which means **no limit** (the sentinel mirrors `-start`'s `0`-default). The output is the intersection of the three range flags — `start <= index <= min(start+n-1, limit)`, at most `-n` lines — so e.g. `gofib -n 100000 -limit 5` prints exactly 5 lines, instantly. `-limit < start` (both positive) is **not an error**: gofib prints nothing and exits `0`. Must be `>= 0`, otherwise gofib exits non-zero with an error on stderr (`invalid value -1 for flag -limit: must be >= 0`). Combines with `-json` (one object per surviving index) and `-pretty` (columns sized to the capped range, not `start+n-1`). |
| `-seed <int>` | Single-index lookup: print only the Fibonacci number at this index, one line, in the active output mode. Default `0` means unset (the sentinel mirrors `-start`'s `0`-default), so plain `gofib` output is unchanged. A positive `-seed` **overrides `-n`, `-start`, and `-limit` for output** (their validation still applies) — e.g. `gofib -seed 10 -n 5 -start 2 -limit 3` prints exactly `10: 55` and nothing else. Must be `>= 0`, otherwise gofib exits non-zero with an error on stderr (`invalid value -1 for flag -seed: must be >= 0`). Combines with `-json` (`gofib -seed 7 -json` prints `{"index":7,"fib":"13"}`) and `-pretty` (columns sized to the single row). |
| `-json` | JSON mode: emit JSON Lines instead of text — one `{"index":<int>,"fib":"<string>"}` object per number. `fib` is a string by decision (ADR-0001), since F(100) overflows int64. Never a JSON array. |
| `-pretty` | Aligned column output in text mode: both columns right-aligned, sized to the largest index and value printed. **No effect with `-json`** — JSON Lines have no columns to align. |
| `-version` | Prints exactly `gofib 1.3.0` (from the `Version` const in `main.go`) and takes precedence over every other flag — even an otherwise-invalid combination like `-version -n 0 -json` prints the version and exits `0`. |

`-pretty` and `-json` together is not an error; JSON mode simply wins
and the output is identical to `-json` alone.
