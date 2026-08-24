// Command gofib prints Fibonacci numbers, one per line, prefixed with
// the index: "1: 1", "2: 1", "3: 2", ... By default it prints the first
// 100; the -n flag changes how many are printed, -start changes the
// first index printed (it does not change what -n counts), -limit caps
// the largest index printed (it does not change what -n counts either;
// 0, the default, means no limit), -seed prints only the Fibonacci
// number at a single index (0, the default, means unset), and -format
// selects the output mode globally: text (the default), json (JSON
// Lines), pretty (right-aligned text columns), or table (values only,
// right-justified). -json and -pretty are shortcuts for -format json
// and -format pretty: with -format unset they keep their legacy
// meaning (and plain -json -pretty lets JSON win), but an explicit
// -format must agree with any also-given shortcut. The -version flag
// prints "gofib <Version>" and takes precedence over every other flag.
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

// outputMode is the global output style selected by -format (with
// -json and -pretty acting as legacy shortcuts).
type outputMode int

const (
	modeText outputMode = iota
	modeJSON
	modePretty
	modeTable
)

// modeName returns the -format spelling of an output mode.
func modeName(m outputMode) string {
	switch m {
	case modeJSON:
		return "json"
	case modePretty:
		return "pretty"
	case modeTable:
		return "table"
	default:
		return "text"
	}
}

// parseMode validates a -format value: the empty string (unset) and
// "text" both select the default text mode; anything outside the four
// modes is rejected with the flag-package-style error naming the valid
// values.
func parseMode(format string) (outputMode, error) {
	switch format {
	case "", "text":
		return modeText, nil
	case "json":
		return modeJSON, nil
	case "pretty":
		return modePretty, nil
	case "table":
		return modeTable, nil
	default:
		return modeText, fmt.Errorf("invalid value %q for flag -format: must be one of text, json, pretty, table", format)
	}
}

// resolveMode decides the active output mode from the -format value and
// the -json/-pretty shortcuts. With -format unset the shortcuts keep
// their legacy meaning — JSON wins over pretty, plain flags change
// nothing. With -format explicitly set (any non-empty value, including
// text and table) every also-given shortcut must agree with it;
// disagreement is an error naming both flags.
func resolveMode(format string, asJSON, pretty bool) (outputMode, error) {
	mode, err := parseMode(format)
	if err != nil {
		return modeText, err
	}
	if format == "" {
		if asJSON {
			return modeJSON, nil
		}
		if pretty {
			return modePretty, nil
		}
		return modeText, nil
	}
	if asJSON && mode != modeJSON {
		return modeText, fmt.Errorf("flags -json and -format conflict: -json selects json but -format selects %s", modeName(mode))
	}
	if pretty && mode != modePretty {
		return modeText, fmt.Errorf("flags -pretty and -format conflict: -pretty selects pretty but -format selects %s", modeName(mode))
	}
	return mode, nil
}

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
// one JSON object {"index": <int>, "fib": "<value>"} (JSON Lines).
// In table mode each line is the value alone, right-justified to the
// width of the largest value printed (no index column). The mode comes
// from format plus the -json/-pretty shortcuts via resolveMode: with
// format unset the shortcuts keep their legacy JSON-wins meaning; with
// format set (including text and table) a conflicting shortcut is an
// error naming both flags, and an invalid format value is rejected.
// With version set it prints only the single
// line "gofib <Version>" and every other flag (including an invalid
// count, start, limit, seed, or format, or a shortcut/format conflict)
// is ignored. A positive seed selects
// lookup mode: only the single index seed prints (in the active
// output mode, with -pretty sized from that sole index), it overrides
// -n, -start, and -limit, and the range-flag validation is skipped —
// like -version, an invalid range flag alongside a positive seed is
// ignored. A negative seed is rejected; seed 0 is the unset sentinel.
// Otherwise run returns an error when count < 1, start < 0, or
// limit < 0.
func run(w io.Writer, start, count, limit, seed int, format string, asJSON, pretty, version bool) error {
	if version {
		fmt.Fprintf(w, "gofib %s\n", Version)
		return nil
	}
	mode, err := resolveMode(format, asJSON, pretty)
	if err != nil {
		return err
	}
	if seed < 0 {
		return fmt.Errorf("invalid value %d for flag -seed: must be >= 0", seed)
	}
	if seed > 0 {
		// Lookup mode: exactly one entry for index seed, regardless
		// of the range flags (whose validation is skipped, mirroring
		// -version's ignore-invalid semantics).
		start, count, limit = seed, 1, 0
	} else {
		if count < 1 {
			return fmt.Errorf("invalid value %d for flag -n: must be >= 1", count)
		}
		if start < 0 {
			return fmt.Errorf("invalid value %d for flag -start: must be >= 0", start)
		}
		if limit < 0 {
			return fmt.Errorf("invalid value %d for flag -limit: must be >= 0", limit)
		}
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
	// Pretty and table text modes size their columns from the largest
	// index and value printed: last and Fib(last).
	idxW, valW := 0, 0
	switch mode {
	case modePretty:
		idxW = len(strconv.Itoa(last))
		valW = len(Fib(last).String())
	case modeTable:
		valW = len(Fib(last).String())
	}
	for i := start; i <= last; i++ {
		switch mode {
		case modeJSON:
			if err := enc.Encode(fibLine{Index: i, Fib: Fib(i).String()}); err != nil {
				return err
			}
		case modePretty:
			fmt.Fprintf(w, "%*d: %*s\n", idxW, i, valW, Fib(i).String())
		case modeTable:
			fmt.Fprintf(w, "%*s\n", valW, Fib(i).String())
		default:
			fmt.Fprintf(w, "%d: %v\n", i, Fib(i))
		}
	}
	return nil
}

func main() {
	n := flag.Int("n", defaultCount, "how many Fibonacci numbers to print (must be >= 1; default 100)")
	start := flag.Int("start", 0, "index of the first Fibonacci number to print (must be >= 0; 0 starts at 1 like the default)")
	limit := flag.Int("limit", 0, "largest index to print (must be >= 0; 0 means no limit)")
	seed := flag.Int("seed", 0, "print only the Fibonacci number at this index, overriding -n, -start, and -limit (must be >= 0; 0 means unset)")
	format := flag.String("format", "", "output mode: text, json, pretty, or table (default text); must agree with -json/-pretty when those are also set")
	asJSON := flag.Bool("json", false, "shortcut for -format json: emit JSON Lines instead of text (one {\"index\": i, \"fib\": \"value\"} object per number)")
	pretty := flag.Bool("pretty", false, "shortcut for -format pretty: align text output into two right-aligned columns sized to the largest index and value")
	version := flag.Bool("version", false, "print \"gofib <version>\" and exit; takes precedence over all other flags")
	flag.Parse()
	if err := run(os.Stdout, *start, *n, *limit, *seed, *format, *asJSON, *pretty, *version); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
