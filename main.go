// Command gofib prints Fibonacci numbers, one per line, prefixed with
// the index: "1: 1", "2: 1", "3: 2", ... By default it prints the first
// 100; the -n flag changes how many are printed, -start changes the
// first index printed (it does not change what -n counts), -json
// switches to JSON Lines output, and -pretty aligns the text columns
// (it has no effect with -json). The -version flag prints
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

// run writes count Fibonacci numbers to w, beginning at index start
// (an unset -start value of 0 means the historical default of 1), so
// it prints the indices start..start+count-1 and -start never changes
// what count counts. In text mode each line is "<index>: <value>";
// with pretty both columns are right-aligned, the index to the width
// of the largest index printed and the value to the width of the
// largest value printed, separated by ": ". In JSON mode each line is
// one JSON object {"index": <int>, "fib": "<value>"} (JSON Lines) and
// pretty has no effect. With version set it prints only the single
// line "gofib <Version>" and every other flag (including an invalid
// count or start) is ignored. Otherwise it returns an error when
// count < 1 or start < 0.
func run(w io.Writer, start, count int, asJSON, pretty, version bool) error {
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
	if start == 0 {
		start = defaultStart
	}
	last := start + count - 1
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
	asJSON := flag.Bool("json", false, "emit JSON Lines instead of text: one {\"index\": i, \"fib\": \"value\"} object per number")
	pretty := flag.Bool("pretty", false, "align text output into two right-aligned columns sized to the largest index and value (no effect with -json)")
	version := flag.Bool("version", false, "print \"gofib <version>\" and exit; takes precedence over all other flags")
	flag.Parse()
	if err := run(os.Stdout, *start, *n, *asJSON, *pretty, *version); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
