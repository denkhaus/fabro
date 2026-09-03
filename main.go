// Command gofib prints Fibonacci numbers, one per line, prefixed with
// the index: "1: 1", "2: 1", "3: 2", ... By default it prints the first
// 100; the -n flag changes how many are printed, -start changes the
// first index printed (it does not change what -n counts), -limit caps
// the largest index printed (it does not change what -n counts either;
// 0, the default, means no limit), -step strides the selected range so
// only every step-th number prints, starting with the first (indices
// start, start+k, start+2k, ... up to last; 1, the default, prints
// every number; it must be >= 1 and is ignored, not an error, when a
// positive -seed is set), -last keeps only the last k numbers of the
// already-selected, already-stepped selection (the pipeline order is
// selection via -n/-start/-limit, then -step stride, then -last tail;
// 0, the default, means unset, a selection shorter than k is clamped
// to the whole selection rather than an error, and a positive -last
// conflicts with a positive -seed the way -sum does, while -step is
// merely ignored under -seed), -seed prints only the Fibonacci
// number at a single index (0, the default, means unset), and -format
// selects the output mode globally: text (the default), json (JSON
// Lines), pretty (right-aligned text columns), table (values only,
// right-justified), or csv (one plain <index>,<fib> CSV record per
// line, no header). -sum replaces the per-number output with a single
// line carrying the big.Int sum of the Fibonacci numbers in the same
// -start/-limit/-n intersection line mode would print (an empty range
// sums to 0); it cannot be combined with a positive -seed. -json and -pretty are shortcuts for -format json
// and -format pretty: with -format unset they keep their legacy
// meaning (and plain -json -pretty lets JSON win), but an explicit
// -format must agree with any also-given shortcut. The -o flag
// redirects all output (any mode, -sum, -version) to a file instead of
// stdout; the default empty value means stdout. The -version flag
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

// sumLine is the single JSON object emitted in -sum -json mode. The
// sum is a string because it can exceed int64; index_range carries the
// computed effective bounds [start, capped last] of the selected range.
type sumLine struct {
	IndexRange []int  `json:"index_range"`
	Sum        string `json:"sum"`
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
	modeCSV
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
	case modeCSV:
		return "csv"
	default:
		return "text"
	}
}

// parseMode validates a -format value: the empty string (unset) and
// "text" both select the default text mode; anything outside the five
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
	case "csv":
		return modeCSV, nil
	default:
		return modeText, fmt.Errorf("invalid value %q for flag -format: must be one of text, json, pretty, table, csv", format)
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

// options holds every flag value gofib accepts. parseOptions is the
// single place that fills it; adding a new flag (e.g. a future -o
// output path) means adding one field here, one Var registration, and
// at most one validation rule in parseOptions.
type options struct {
	n       int
	start   int
	limit   int
	step    int
	last    int
	seed    int
	format  string
	json    bool
	pretty  bool
	version bool
	sum     bool
	output  string
}

// parseOptions registers gofib's flags, parses args, and applies every
// validation rule gofib has; it is the single rejection point for bad
// flag values. Validation order is fixed and behavior-preserving:
//
//   - -version short-circuits before any validation: when it is set,
//     parseOptions returns the options as parsed with no error, so an
//     invalid count, start, limit, seed, or format alongside -version
//     is ignored (run still prints only the version line).
//   - mode resolution: an invalid -format value, then any
//     shortcut/format disagreement, is rejected.
//   - a negative -seed is rejected.
//   - a positive -seed combined with -sum is rejected.
//   - a positive -seed combined with a positive -last is rejected.
//   - range checks (-n >= 1, -start >= 0, -limit >= 0, -step >= 1,
//     -last >= 0)
//     run only when
//     -seed is unset (0): a positive -seed overrides the range flags,
//     whose values are then ignored, mirroring -version's
//     ignore-invalid semantics.
//
// Flag-syntax errors (e.g. -n=abc) are not returned as errors: the
// flag set uses the flag package's ExitOnError behavior (usage on
// stderr, exit 2), exactly as before.
func parseOptions(args []string) (options, error) {
	var opts options
	fs := flag.NewFlagSet("gofib", flag.ExitOnError)
	fs.IntVar(&opts.n, "n", defaultCount, "how many Fibonacci numbers to print (must be >= 1; default 100)")
	fs.IntVar(&opts.start, "start", 0, "index of the first Fibonacci number to print (must be >= 0; 0 starts at 1 like the default)")
	fs.IntVar(&opts.limit, "limit", 0, "largest index to print (must be >= 0; 0 means no limit)")
	fs.IntVar(&opts.step, "step", 1, "print only every step-th number of the selected range, starting with the first (must be >= 1; default 1 prints every number; ignored when -seed is positive)")
	fs.IntVar(&opts.last, "last", 0, "print only the last k numbers of the selected range, applied after -n/-start/-limit selection and -step stride (must be >= 1; 0 means unset; clamps to the selection size; conflicts with a positive -seed)")
	fs.IntVar(&opts.seed, "seed", 0, "print only the Fibonacci number at this index, overriding -n, -start, and -limit (must be >= 0; 0 means unset)")
	fs.StringVar(&opts.format, "format", "", "output mode: text, json, pretty, table, or csv (default text); must agree with -json/-pretty when those are also set")
	fs.BoolVar(&opts.json, "json", false, `shortcut for -format json: emit JSON Lines instead of text (one {"index": i, "fib": "value"} object per number)`)
	fs.BoolVar(&opts.pretty, "pretty", false, "shortcut for -format pretty: align text output into two right-aligned columns sized to the largest index and value")
	fs.BoolVar(&opts.version, "version", false, "print \"gofib <version>\" and exit; takes precedence over all other flags")
	fs.BoolVar(&opts.sum, "sum", false, "print the sum of the Fibonacci numbers in the selected range instead of the numbers themselves (cannot be combined with a positive -seed)")
	fs.StringVar(&opts.output, "o", "", "write output to this file instead of stdout (default empty = stdout)")
	fs.Parse(args)
	if opts.version {
		return opts, nil
	}
	if _, err := resolveMode(opts.format, opts.json, opts.pretty); err != nil {
		return opts, err
	}
	if opts.seed < 0 {
		return opts, fmt.Errorf("invalid value %d for flag -seed: must be >= 0", opts.seed)
	}
	if opts.seed > 0 && opts.sum {
		return opts, fmt.Errorf("flags -seed and -sum conflict: -seed prints a single index but -sum prints the range sum")
	}
	if opts.seed > 0 && opts.last > 0 {
		return opts, fmt.Errorf("flags -seed and -last conflict: -seed prints a single index but -last tails the range")
	}
	if opts.seed == 0 {
		if opts.n < 1 {
			return opts, fmt.Errorf("invalid value %d for flag -n: must be >= 1", opts.n)
		}
		if opts.start < 0 {
			return opts, fmt.Errorf("invalid value %d for flag -start: must be >= 0", opts.start)
		}
		if opts.limit < 0 {
			return opts, fmt.Errorf("invalid value %d for flag -limit: must be >= 0", opts.limit)
		}
		if opts.step < 1 {
			return opts, fmt.Errorf("invalid value %d for flag -step: must be >= 1", opts.step)
		}
		if opts.last < 0 {
			return opts, fmt.Errorf("invalid value %d for flag -last: must be >= 1 or 0 to unset", opts.last)
		}
	}
	return opts, nil
}

// run writes the Fibonacci output selected by opts to w. It performs no
// flag validation: parseOptions has already rejected every bad value,
// so run only renders. With version set it prints only the single line
// "gofib <Version>" and ignores every other flag. A positive seed
// selects lookup mode: only the single index seed prints (in the active
// output mode, with -pretty sized from that sole index), overriding
// -n, -start, and -limit, and ignoring -step. A positive -step k
// strides the selected range so only every k-th number prints,
// starting with the first; -sum then sums the numbers that remain
// after stepping while its reported range bounds stay [start, last].
// A positive -last k tails the stepped selection: only the last k
// members of start, start+step, ... <= last print (and -sum sums only
// those), clamped to the whole selection when it has fewer than k
// members; the reported range bounds still stay [start, last].
// With sum set, run prints exactly one line
// carrying the big.Int sum of the Fibonacci numbers in the same
// selected range instead of the per-number output. An empty selected
// range (e.g. limit below start) sums to 0 and prints zero lines in
// line mode — never an error.
func run(w io.Writer, opts options) error {
	if opts.version {
		fmt.Fprintf(w, "gofib %s\n", Version)
		return nil
	}
	// parseOptions has already validated format and the shortcuts, so
	// this can never fail; the error return is purely defensive.
	mode, err := resolveMode(opts.format, opts.json, opts.pretty)
	if err != nil {
		return err
	}
	start, count, limit := opts.start, opts.n, opts.limit
	if opts.seed > 0 {
		// Lookup mode: exactly one entry for index seed, regardless
		// of the range flags.
		start, count, limit = opts.seed, 1, 0
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
	// -step strides the selection: only every step-th number of the
	// range prints, starting with the first. A positive -seed ignores
	// -step (and its validation is skipped under -seed, so a zero or
	// negative stride is defensively clamped to 1 to keep the loops
	// terminating).
	step := opts.step
	if opts.seed > 0 || step < 1 {
		step = 1
	}
	// steppedLast is the largest index actually printed: the last
	// member of the arithmetic sequence start, start+step, ... <= last.
	// Pretty/table column widths are sized from it, not from last.
	steppedLast := last
	if step > 1 && last >= start {
		steppedLast = start + ((last-start)/step)*step
	}
	// -last tails the stepped selection: iteration advances past the
	// first (count-k) members so exactly the last k remain. The loop
	// bound stays `last` (and steppedLast, which tails keep, still
	// sizes the pretty/table columns); a selection shorter than k is
	// clamped to the whole selection, not an error. A positive -seed
	// conflicts with -last in parseOptions, and its validation is
	// skipped under -seed, so a negative k is defensively ignored
	// here.
	first := start
	if opts.last > 0 && last >= start {
		count := (steppedLast-start)/step + 1
		if opts.last < count {
			first = steppedLast - (opts.last-1)*step
		}
	}
	enc := json.NewEncoder(w)
	if opts.sum {
		// Sum mode: one line with the big.Int total of the stepped
		// selection instead of the per-number output. The reported
		// bounds stay the full effective range [start, last]: stepping
		// and tailing filter the selection, not the range. An empty range
		// (start > last) leaves the total at 0 — not an error.
		total := new(big.Int)
		for i := first; i <= last; i += step {
			total.Add(total, Fib(i))
		}
		switch mode {
		case modeJSON:
			return enc.Encode(sumLine{IndexRange: []int{start, last}, Sum: total.String()})
		case modeCSV:
			fmt.Fprintf(w, "sum,%d,%d,%s\n", start, last, total)
		case modePretty, modeTable:
			fmt.Fprintf(w, "%s\n", total)
		default:
			fmt.Fprintf(w, "sum: %v\n", total)
		}
		return nil
	}
	// Pretty and table text modes size their columns from the largest
	// index and value actually printed: steppedLast and Fib(steppedLast).
	idxW, valW := 0, 0
	switch mode {
	case modePretty:
		idxW = len(strconv.Itoa(steppedLast))
		valW = len(Fib(steppedLast).String())
	case modeTable:
		valW = len(Fib(steppedLast).String())
	}
	for i := first; i <= last; i += step {
		switch mode {
		case modeJSON:
			if err := enc.Encode(fibLine{Index: i, Fib: Fib(i).String()}); err != nil {
				return err
			}
		case modePretty:
			fmt.Fprintf(w, "%*d: %*s\n", idxW, i, valW, Fib(i).String())
		case modeTable:
			fmt.Fprintf(w, "%*s\n", valW, Fib(i).String())
		case modeCSV:
			fmt.Fprintf(w, "%d,%s\n", i, Fib(i).String())
		default:
			fmt.Fprintf(w, "%d: %v\n", i, Fib(i))
		}
	}
	return nil
}

// resolveOutput maps the -o value to a writer. An empty path means
// stdout, returned unclosable (closing stdout is main's job only if it
// opened the file itself). Any other path is created or truncated; a
// failure to open is reported with the gofib-prefixed message callers
// print on stderr.
func resolveOutput(path string) (io.WriteCloser, error) {
	if path == "" {
		return nil, nil
	}
	f, err := os.Create(path)
	if err != nil {
		return nil, fmt.Errorf("gofib: cannot open %s: %v", path, err)
	}
	return f, nil
}

func main() {
	opts, err := parseOptions(os.Args[1:])
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	w := io.Writer(os.Stdout)
	out, err := resolveOutput(opts.output)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	if out != nil {
		defer out.Close()
		w = out
	}
	if err := run(w, opts); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
