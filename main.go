// Command gofib prints Fibonacci numbers, one per line, prefixed with
// the index: "1: 1", "2: 1", "3: 2", ... By default it prints the first
// 100; the -n flag changes how many are printed, -start changes the
// first index printed (it does not change what -n counts), -limit caps
// the largest index printed (it does not change what -n counts either;
// 0, the default, means no limit), -json switches to JSON Lines
// output, and -pretty aligns the text columns (it has no effect with
// -json). The -seed flag prints only the Fibonacci number at one index
// (0, the default, means unset), overriding -n, -start, and -limit for
// output while their validation is unchanged. The -version flag prints
// "gofib <Version>" and takes precedence over every other flag.
package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"math/big"
	"os"
	"strconv"
)

// defaultCount is how many numbers gofib prints when -n is omitted.
const defaultCount = 100

// Version is gofib's semantic version, reported by the -version flag.
const Version = "1.3.0"

// fibLine is the JSON object emitted once per number in -json mode.
// The Fibonacci value is a string because it can exceed int64.
type fibLine struct {
	Index int    `json:"index"`
	Fib   string `json:"fib"`
}

// Fib returns the n-th Fibonacci number, with F(1) = F(2) = 1.
func Fib(n int) *big.Int {
	a, b := big.NewInt(0), big.NewInt(1)
	for i := 0; i < n; i++ {
		a, b = b, new(big.Int).Add(a, b)
	}
	return a
}

// defaultStart is the effective first index when -start is omitted or
// zero: gofib's indices are 1-based, and an unset (or explicit 0)
// -start preserves the historical output starting at index 1.
const defaultStart = 1

// run writes at most count Fibonacci numbers to w, beginning at index
// start (an unset -start value of 0 means the historical default of
// 1), so it prints the indices start..start+count-1 and neither
// -start nor -limit changes what count counts: a positive limit caps
// the largest index printed (an unset -limit of 0 means no limit),
// shrinking the range to start..min(start+count-1, limit), and a
// limit below start leaves the range empty — zero lines, not an
// error. In text mode each line is "<index>: <value>";
// with pretty both columns are right-aligned, the index to the width
// of the largest index printed and the value to the width of the
// largest value printed (both sized from the limit-capped last
// index), separated by ": ". In JSON mode each line is
// one JSON object {"index": <int>, "fib": "<value>"} (JSON Lines) and
// pretty has no effect. With version set it prints only the single
// line "gofib <Version>" and every other flag (including an invalid
// count, start, or limit) is ignored. Otherwise it returns an error
// when count < 1, start < 0, limit < 0, or seed < 0. A positive seed
// overrides the range flags for output: exactly one entry for index
// seed is printed in the active output mode (in pretty mode the
// columns are sized to that single row), while -n, -start, and -limit
// keep their usual validation; seed 0 is the unset sentinel, so plain
// output is unchanged.
func run(w io.Writer, start, count, limit, seed int, asJSON, pretty, version bool) error {
	if version {
		fmt.Fprintf(w, "gofib %s\n", Version)
		return nil
	}
	if count < 1 {
		return fmt.Errorf("invalid value %d for flag -n: must be >= 1", count)
	}
	if start < 0 {
		return fmt.Errorf("invalid value %d for flag -start: must be >= 0", start)
	}
	if limit < 0 {
		return fmt.Errorf("invalid value %d for flag -limit: must be >= 0", limit)
	}
	if seed < 0 {
		return fmt.Errorf("invalid value %d for flag -seed: must be >= 0", seed)
	}
	// A positive -seed is a single-index lookup: print exactly one
	// entry for that index in the active output mode, ignoring -n,
	// -start, and -limit for output (they were validated above).
	if seed > 0 {
		if asJSON {
			return json.NewEncoder(w).Encode(fibLine{Index: seed, Fib: Fib(seed).String()})
		}
		if pretty {
			fmt.Fprintf(w, "%*d: %*s\n",
				len(strconv.Itoa(seed)), seed,
				len(Fib(seed).String()), Fib(seed).String())
		} else {
			fmt.Fprintf(w, "%d: %v\n", seed, Fib(seed))
		}
		return nil
	}
	if start == 0 {
		start = defaultStart
	}
	last := start + count - 1
	// A positive -limit caps the largest index printed; 0 (the
	// default) is the "no limit" sentinel, mirroring -start's 0. When
	// the cap lands below start, the loop below runs zero times:
	// empty output, not an error.
	if limit > 0 && limit < last {
		last = limit
	}
	enc := json.NewEncoder(w)
	// Pretty text mode sizes its columns from the largest index and
	// value printed: last and Fib(last).
	idxW, valW := 0, 0
	if pretty && !asJSON {
		idxW = len(strconv.Itoa(last))
		valW = len(Fib(last).String())
	}
	for i := start; i <= last; i++ {
		if asJSON {
			if err := enc.Encode(fibLine{Index: i, Fib: Fib(i).String()}); err != nil {
				return err
			}
		} else if pretty {
			fmt.Fprintf(w, "%*d: %*s\n", idxW, i, valW, Fib(i).String())
		} else {
			fmt.Fprintf(w, "%d: %v\n", i, Fib(i))
		}
	}
	return nil
}

func main() {
	n := flag.Int("n", defaultCount, "how many Fibonacci numbers to print (must be >= 1; default 100)")
	start := flag.Int("start", 0, "index of the first Fibonacci number to print (must be >= 0; 0 starts at 1 like the default)")
	limit := flag.Int("limit", 0, "largest index to print (must be >= 0; 0 means no limit)")
	asJSON := flag.Bool("json", false, "emit JSON Lines instead of text: one {\"index\": i, \"fib\": \"value\"} object per number")
	pretty := flag.Bool("pretty", false, "align text output into two right-aligned columns sized to the largest index and value (no effect with -json)")
	seed := flag.Int("seed", 0, "print only the Fibonacci number at this index (must be >= 0; overrides -n/-start/-limit for output; 0 means unset)")
	version := flag.Bool("version", false, "print \"gofib <version>\" and exit; takes precedence over all other flags")
	flag.Parse()
	if err := run(os.Stdout, *start, *n, *limit, *seed, *asJSON, *pretty, *version); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
