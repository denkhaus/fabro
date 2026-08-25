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

`-seed i` switches gofib into lookup mode: print **only** the
Fibonacci number at index `i`, exactly one line, in the active output
mode. It overrides the range flags (`-n`, `-start`, `-limit`), and —
like `-version` — it makes their validation a no-op, so even an
otherwise-invalid combination still prints the single entry:

```
$ gofib -seed 10 -n 5 -start 2
10: 55

$ gofib -seed 10 -json
{"index":10,"fib":"55"}
```

`-format <mode>` selects the output style globally: `text` (the
default), `json`, `pretty`, or the compact `table` mode (values only,
one per line, right-justified to the width of the widest value
printed, no index column). It composes with `-start`, `-n`, `-limit`,
and `-seed` as usual:

```
$ gofib -n 5 -format text
1: 1
2: 1
3: 2
4: 3
5: 5

$ gofib -n 3 -format json
{"index":1,"fib":"1"}
{"index":2,"fib":"1"}
{"index":3,"fib":"2"}

$ gofib -n 12 -format pretty
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

$ gofib -n 12 -format table
  1
  1
  2
  3
  5
  8
 13
 21
 34
 55
 89
144

$ gofib -start 8 -n 5 -format table
 21
 34
 55
 89
144

$ gofib -seed 10 -format table
55
```

`-json` and `-pretty` are **shortcuts**: `-json` behaves exactly like
`-format json` and `-pretty` exactly like `-format pretty`. With
`-format` unset, plain `-json`/`-pretty` behavior is unchanged
(including `-json -pretty`, where JSON wins and there is no error).
But when `-format` is **explicitly set** (including to `text` or
`table`), any also-given shortcut must agree — a disagreement is an
error naming both flags:

```
$ gofib -pretty -format json
flags -pretty and -format conflict: -pretty selects pretty but -format selects json
$ echo $?
1

$ gofib -format xml
invalid value "xml" for flag -format: must be one of text, json, pretty, table
$ echo $?
1
```

Agreeing combinations (`-pretty -format pretty`, `-json -format
json`) are accepted. `-version` still wins over everything — even an
invalid `-format` value or a shortcut/format conflict:

```
$ gofib -version -format xml
gofib 1.3.0
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

`-sum` replaces the per-number output with a single line carrying the
`math/big` sum of the Fibonacci numbers in the same `-start`/`-limit`/
`-n` intersection line mode would print (an empty range sums to `0`),
rendered per output mode — `sum: <value>` in text, one JSON object in
json, the bare value in pretty/table:

```
$ gofib -n 10 -sum
sum: 143
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
| `-seed <int>` | Lookup flag: print **only** the Fibonacci number at this index — exactly one line in the active output mode (`i: <value>`, or one JSON object, or a single `-pretty` row sized from that sole index). Default `0` is the unset **sentinel** (mirroring `-start`): plain `gofib` output is unchanged. **Precedence**: a positive `-seed` overrides `-n`, `-start`, and `-limit` so only index `i` prints — e.g. `gofib -seed 10 -n 5 -start 2` prints exactly `10: 55` — and, like `-version`, it skips the range-flag validation, so an otherwise-invalid combination such as `-seed 10 -n 0` still prints the single entry. `-version` still wins over everything. Must be `>= 0`, otherwise gofib exits non-zero with an error on stderr (`invalid value -1 for flag -seed: must be >= 0`). |
| `-format <mode>` | Output-mode flag: selects the output style globally — `text` (the default), `json` (JSON Lines), `pretty` (right-aligned text columns), or `table` (compact: values only, one per line, right-justified to the width of the widest value printed, no index column). Composes with `-start`, `-n`, `-limit`, and `-seed` as usual. An invalid value exits non-zero with `invalid value "xml" for flag -format: must be one of text, json, pretty, table` on stderr. `-json` and `-pretty` are **shortcuts** for `-format json` / `-format pretty`: with `-format` unset they keep their legacy meaning (plain `-json -pretty` is still not an error — JSON wins), but an explicitly set `-format` must **agree** with any also-given shortcut, so e.g. `-pretty -format json` exits non-zero with an error naming both flags (agreeing combos like `-json -format json` are fine). `-version` still wins over everything, including an invalid or conflicting `-format`. |
| `-json` | JSON mode: emit JSON Lines instead of text — one `{"index":<int>,"fib":"<string>"}` object per number. `fib` is a string by decision (ADR-0001), since F(100) overflows int64. Never a JSON array. Shortcut for `-format json`; conflicts with an explicit `-format` selecting another mode. |
| `-pretty` | Aligned column output in text mode: both columns right-aligned, sized to the largest index and value printed. Shortcut for `-format pretty`; conflicts with an explicit `-format` selecting another mode. |
| `-sum` | Sum flag: print exactly one line with the `math/big` sum of the Fibonacci numbers in the same `-start`/`-limit`/`-n` intersection line mode would print — text mode prints `sum: <value>`, json mode one object `{"index_range":[first,last],"sum":"<string>"}` (sum as string since it can exceed int64; `index_range` reports the computed effective bounds even when the range is empty/inverted), pretty/table print the bare value + newline. An empty selected range (`-limit < -start`) sums to `0`, not an error. A positive `-seed` with `-sum` exits non-zero with an error naming both flags; `-seed 0` (unset sentinel) is legal. `-version` still wins. |
| `-version` | Prints exactly `gofib 1.3.0` (from the `Version` const in `main.go`) and takes precedence over every other flag — even an otherwise-invalid combination like `-version -n 0 -json` prints the version and exits `0`. |

With no `-format`, plain `-pretty` and `-json` together is not an
error; JSON mode simply wins and the output is identical to `-json`
alone.
