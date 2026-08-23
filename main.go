// Command gofib prints Fibonacci numbers, one per line, prefixed with
// the index: "1: 1", "2: 1", "3: 2", ... By default it prints the first
// 100; the -n flag changes how many are printed, -json switches to
// JSON Lines output, and -pretty aligns the text columns (it has no
// effect with -json). The -version flag prints the gofib version and
// exits, taking precedence over all other output modes.
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

// Version is the gofib release version printed by -version.
const Version = "1.3.0"

// defaultCount is how many numbers gofib prints when -n is omitted.
const defaultCount = 100

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

// run writes the first count Fibonacci numbers to w. In text mode each
// line is "<index>: <value>"; with pretty both columns are right-aligned,
// the index to the width of the largest index printed and the value to
// the width of the largest value printed, separated by ": ". In JSON
// mode each line is one JSON object {"index": <int>, "fib": "<value>"}
// (JSON Lines) and pretty has no effect. With version it prints only the
// single line "gofib <Version>" — this takes precedence over every other
// mode, including the count validation. It returns an error when
// count < 1 (and version is not set).
func run(w io.Writer, count int, asJSON, pretty, version bool) error {
	if version {
		_, err := fmt.Fprintln(w, "gofib", Version)
		return err
	}
	if count < 1 {
		return fmt.Errorf("invalid value %d for flag -n: must be >= 1", count)
	}
	enc := json.NewEncoder(w)
	// Pretty text mode sizes its columns from the largest index and
	// value printed: count and Fib(count).
	idxW, valW := 0, 0
	if pretty && !asJSON {
		idxW = len(strconv.Itoa(count))
		valW = len(Fib(count).String())
	}
	for i := 1; i <= count; i++ {
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
	asJSON := flag.Bool("json", false, "emit JSON Lines instead of text: one {\"index\": i, \"fib\": \"value\"} object per number")
	pretty := flag.Bool("pretty", false, "align text output into two right-aligned columns sized to the largest index and value (no effect with -json)")
	version := flag.Bool("version", false, "print the gofib version and exit (takes precedence over all other output modes)")
	flag.Parse()
	if err := run(os.Stdout, *n, *asJSON, *pretty, *version); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
