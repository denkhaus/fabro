package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"math/big"
	"strconv"
	"strings"
	"testing"
)

// opts builds an options value in the old run-argument order (start,
// n, limit, seed, format, json, pretty, version, sum), keeping the
// table-driven call sites compact.
func opts(start, n, limit, seed int, format string, asJSON, pretty, version, sum bool) options {
	return options{n: n, start: start, limit: limit, seed: seed, format: format, json: asJSON, pretty: pretty, version: version, sum: sum}
}

func mustBig(s string) *big.Int {
	n, ok := new(big.Int).SetString(s, 10)
	if !ok {
		panic("invalid big.Int literal: " + s)
	}
	return n
}

func TestFib(t *testing.T) {
	tests := []struct {
		n    int
		want *big.Int
	}{
		{1, big.NewInt(1)},
		{2, big.NewInt(1)},
		{10, big.NewInt(55)},
		// F(100) exceeds int64; value must be built from a string.
		{100, mustBig("354224848179261915075")},
	}
	for _, tt := range tests {
		if got := Fib(tt.n); got.Cmp(tt.want) != 0 {
			t.Errorf("Fib(%d) = %v, want %v", tt.n, got, tt.want)
		}
	}
}

func TestRun(t *testing.T) {
	tests := []struct {
		name      string
		n         int
		wantLines int
		wantFirst string
		wantLast  string
	}{
		{"n=1 prints one number", 1, 1, "1: 1", "1: 1"},
		{"n=10 prints ten numbers", 10, 10, "1: 1", "10: 55"},
		// The default path must keep printing the first 100 numbers;
		// F(100) exceeds int64, so its value comes from a string literal.
		{"default prints 100 numbers", defaultCount, 100,
			"1: 1", "100: " + mustBig("354224848179261915075").String()},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var buf bytes.Buffer
			if err := run(&buf, opts(0, tt.n, 0, 0, "", false, false, false, false)); err != nil {
				t.Fatalf("run(%d) returned error: %v", tt.n, err)
			}
			lines := strings.Split(strings.TrimSuffix(buf.String(), "\n"), "\n")
			if len(lines) != tt.wantLines {
				t.Fatalf("run(%d) printed %d lines, want %d", tt.n, len(lines), tt.wantLines)
			}
			if lines[0] != tt.wantFirst {
				t.Errorf("first line = %q, want %q", lines[0], tt.wantFirst)
			}
			if lines[len(lines)-1] != tt.wantLast {
				t.Errorf("last line = %q, want %q", lines[len(lines)-1], tt.wantLast)
			}
		})
	}
}

// TestRunStart pins the -start semantics in text mode: -start s -n k
// prints exactly k lines with the indices s..s+k-1 (s = 0 behaves like
// the historical default starting at 1), so -start never changes what
// -n counts.
func TestRunStart(t *testing.T) {
	tests := []struct {
		name      string
		start     int
		n         int
		wantLines int
		wantFirst string
		wantLast  string
	}{
		{"start 0 behaves like unset", 0, 5, 5, "1: 1", "5: 5"},
		{"start 1 prints indices 1..n", 1, 3, 3, "1: 1", "3: 2"},
		{"start 10 n 5 prints indices 10..14", 10, 5, 5, "10: 55", "14: 377"},
		// -start does not change what -n counts: one line, F(100).
		{"start 100 n 1 prints only F(100)", 100, 1, 1,
			"100: " + mustBig("354224848179261915075").String(),
			"100: " + mustBig("354224848179261915075").String()},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var buf bytes.Buffer
			if err := run(&buf, opts(tt.start, tt.n, 0, 0, "", false, false, false, false)); err != nil {
				t.Fatalf("run(%d, %d) returned error: %v", tt.start, tt.n, err)
			}
			lines := strings.Split(strings.TrimSuffix(buf.String(), "\n"), "\n")
			if len(lines) != tt.wantLines {
				t.Fatalf("run(%d, %d) printed %d lines, want %d", tt.start, tt.n, len(lines), tt.wantLines)
			}
			if lines[0] != tt.wantFirst {
				t.Errorf("first line = %q, want %q", lines[0], tt.wantFirst)
			}
			if lines[len(lines)-1] != tt.wantLast {
				t.Errorf("last line = %q, want %q", lines[len(lines)-1], tt.wantLast)
			}
		})
	}
}

// wantJSONLine returns the canonical JSON line for index i, used to pin
// the exact object shape {"index": <int>, "fib": "<string>"}.
func wantJSONLine(i int) string {
	b, err := json.Marshal(fibLine{Index: i, Fib: Fib(i).String()})
	if err != nil {
		panic("marshal fibLine: " + err.Error())
	}
	return string(b)
}

func TestRunJSON(t *testing.T) {
	tests := []struct {
		name      string
		n         int
		wantLines int
	}{
		{"json n=1 prints one object", 1, 1},
		{"json n=10 prints ten objects", 10, 10},
		// The default path in JSON mode still prints the first 100 numbers.
		{"json default prints 100 objects", defaultCount, 100},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var buf bytes.Buffer
			if err := run(&buf, opts(0, tt.n, 0, 0, "", true, false, false, false)); err != nil {
				t.Fatalf("run(%d, json) returned error: %v", tt.n, err)
			}
			lines := strings.Split(strings.TrimSuffix(buf.String(), "\n"), "\n")
			if len(lines) != tt.wantLines {
				t.Fatalf("run(%d, json) printed %d lines, want %d", tt.n, len(lines), tt.wantLines)
			}
			for i, line := range lines {
				// Unmarshal each line and compare both fields.
				var got fibLine
				if err := json.Unmarshal([]byte(line), &got); err != nil {
					t.Fatalf("line %d is not valid JSON (%q): %v", i+1, line, err)
				}
				if want := wantJSONLine(i + 1); line != want {
					t.Errorf("line %d = %q, want exactly %q", i+1, line, want)
				}
				if got.Index != i+1 {
					t.Errorf("line %d index = %d, want %d", i+1, got.Index, i+1)
				}
				if got.Fib != Fib(i+1).String() {
					t.Errorf("line %d fib = %q, want %q", i+1, got.Fib, Fib(i+1).String())
				}
			}
		})
	}
}

// TestRunStartJSON pins -start in JSON mode: the index field carries
// the actual index (s..s+k-1), fib stays a string, and there is still
// one object per line — never an array.
func TestRunStartJSON(t *testing.T) {
	const start, n = 10, 5
	var buf bytes.Buffer
	if err := run(&buf, opts(start, n, 0, 0, "", true, false, false, false)); err != nil {
		t.Fatalf("run(%d, %d, json) returned error: %v", start, n, err)
	}
	out := buf.String()
	if strings.HasPrefix(out, "[") || strings.HasSuffix(out, "]") {
		t.Errorf("json output looks like an array, want JSON Lines: %q", out)
	}
	lines := strings.Split(strings.TrimSuffix(out, "\n"), "\n")
	if len(lines) != n {
		t.Fatalf("run(%d, %d, json) printed %d lines, want %d", start, n, len(lines), n)
	}
	for i, line := range lines {
		idx := start + i
		var got fibLine
		if err := json.Unmarshal([]byte(line), &got); err != nil {
			t.Fatalf("line %d is not valid JSON (%q): %v", i+1, line, err)
		}
		if want := wantJSONLine(idx); line != want {
			t.Errorf("line %d = %q, want exactly %q", i+1, line, want)
		}
		if got.Index != idx {
			t.Errorf("line %d index = %d, want %d", i+1, got.Index, idx)
		}
		if got.Fib != Fib(idx).String() {
			t.Errorf("line %d fib = %q, want %q", i+1, got.Fib, Fib(idx).String())
		}
	}
}

// prettyLine returns the expected -pretty text line for index i with
// value v: the index right-aligned to the width of the largest index
// (n) and the value right-aligned to the width of the largest value
// (Fib(n)), separated by ": ". Expected lines are computed from this
// width rule; F-values are hardcoded only for small n.
func prettyLine(n, i int, v string) string {
	return fmt.Sprintf("%*d: %*s", len(strconv.Itoa(n)), i, len(Fib(n).String()), v)
}

func TestRunPretty(t *testing.T) {
	// F(1)..F(10); hardcoding F-values is allowed only for small n.
	smallFibs := []string{"1", "1", "2", "3", "5", "8", "13", "21", "34", "55"}
	for _, tt := range []struct {
		name string
		n    int
	}{
		{"pretty n=5 aligns columns by the 5th line", 5},
		{"pretty n=10 aligns columns by the 10th line", 10},
	} {
		t.Run(tt.name, func(t *testing.T) {
			var buf bytes.Buffer
			if err := run(&buf, opts(0, tt.n, 0, 0, "", false, true, false, false)); err != nil {
				t.Fatalf("run(%d, pretty) returned error: %v", tt.n, err)
			}
			lines := strings.Split(strings.TrimSuffix(buf.String(), "\n"), "\n")
			if len(lines) != tt.n {
				t.Fatalf("run(%d, pretty) printed %d lines, want %d", tt.n, len(lines), tt.n)
			}
			for i, line := range lines {
				if want := prettyLine(tt.n, i+1, smallFibs[i]); line != want {
					t.Errorf("line %d = %q, want %q", i+1, line, want)
				}
			}
		})
	}

	// The default path in pretty mode still prints the first 100
	// numbers. The exact last line follows from the width rule alone:
	// index right-aligned to len("100"), value to len(Fib(100)).
	t.Run("pretty default prints 100 aligned numbers", func(t *testing.T) {
		var buf bytes.Buffer
		if err := run(&buf, opts(0, defaultCount, 0, 0, "", false, true, false, false)); err != nil {
			t.Fatalf("run(default, pretty) returned error: %v", err)
		}
		lines := strings.Split(strings.TrimSuffix(buf.String(), "\n"), "\n")
		if len(lines) != defaultCount {
			t.Fatalf("run(default, pretty) printed %d lines, want %d", len(lines), defaultCount)
		}
		want := prettyLine(defaultCount, defaultCount, Fib(defaultCount).String())
		if got := lines[len(lines)-1]; got != want {
			t.Errorf("last line = %q, want %q", got, want)
		}
	})
}

// TestRunStartPretty pins -pretty with -start: the columns are sized to
// the largest index (start+n-1) and the largest value Fib(start+n-1)
// actually printed, not to n.
func TestRunStartPretty(t *testing.T) {
	// F(8)..F(12); hardcoding F-values is allowed only for small n.
	fibs := map[int]string{8: "21", 9: "34", 10: "55", 11: "89", 12: "144"}
	const start, n = 8, 5
	var buf bytes.Buffer
	if err := run(&buf, opts(start, n, 0, 0, "", false, true, false, false)); err != nil {
		t.Fatalf("run(%d, %d, pretty) returned error: %v", start, n, err)
	}
	lines := strings.Split(strings.TrimSuffix(buf.String(), "\n"), "\n")
	if len(lines) != n {
		t.Fatalf("run(%d, %d, pretty) printed %d lines, want %d", start, n, len(lines), n)
	}
	for i, line := range lines {
		idx := start + i
		// Widths come from the largest printed index (12) and the
		// largest printed value (Fib(12) = 144).
		if want := prettyLine(start+n-1, idx, fibs[idx]); line != want {
			t.Errorf("line %d = %q, want %q", i+1, line, want)
		}
	}
}

func TestRunPrettyJSON(t *testing.T) {
	// -pretty only affects text mode: with -json the output must be
	// identical, line for line.
	var jsonOnly, prettyJSON bytes.Buffer
	if err := run(&jsonOnly, opts(0, 3, 0, 0, "", true, false, false, false)); err != nil {
		t.Fatalf("run(3, json) returned error: %v", err)
	}
	if err := run(&prettyJSON, opts(0, 3, 0, 0, "", true, true, false, false)); err != nil {
		t.Fatalf("run(3, json, pretty) returned error: %v", err)
	}
	if prettyJSON.String() != jsonOnly.String() {
		t.Errorf("-pretty -json output = %q, want identical to -json: %q", prettyJSON.String(), jsonOnly.String())
	}
	lines := strings.Split(strings.TrimSuffix(prettyJSON.String(), "\n"), "\n")
	for i, line := range lines {
		if want := wantJSONLine(i + 1); line != want {
			t.Errorf("line %d = %q, want exactly %q", i+1, line, want)
		}
	}
}

func TestRunRejectsInvalidCount(t *testing.T) {
	// Validation lives in parseOptions; run itself never rejects.
	for _, mode := range []struct {
		name   string
		asJSON bool
	}{{"text", false}, {"json", true}} {
		for _, n := range []int{0, -5} {
			args := []string{"-n", strconv.Itoa(n)}
			if mode.asJSON {
				args = append(args, "-json")
			}
			_, err := parseOptions(args)
			if err == nil {
				t.Fatalf("parseOptions(%d, %s) succeeded, want error", n, mode.name)
			}
			if !strings.Contains(err.Error(), "-n") {
				t.Errorf("parseOptions(%d, %s) error %q does not mention the -n flag", n, mode.name, err.Error())
			}
		}
	}
}

// TestRunRejectsInvalidStart pins the -start validation contract: a
// negative start exits non-zero with the exact -n-style error message
// (parseOptions is the rejection point).
func TestRunRejectsInvalidStart(t *testing.T) {
	for _, mode := range []struct {
		name   string
		asJSON bool
	}{{"text", false}, {"json", true}} {
		for _, start := range []int{-1, -5} {
			args := []string{"-start", strconv.Itoa(start)}
			if mode.asJSON {
				args = append(args, "-json")
			}
			_, err := parseOptions(args)
			if err == nil {
				t.Fatalf("parseOptions(%d, %s) succeeded, want error", start, mode.name)
			}
			want := fmt.Sprintf("invalid value %d for flag -start: must be >= 0", start)
			if err.Error() != want {
				t.Errorf("parseOptions(%d, %s) error = %q, want exactly %q", start, mode.name, err.Error(), want)
			}
		}
	}
}

func TestRunVersion(t *testing.T) {
	// -version must win over every other flag combination: the only
	// output is the single line "gofib <Version>". It is checked before
	// the -n, -start, and -limit validation in run(), so even an
	// invalid count, start, or limit still prints the version line and
	// succeeds.
	tests := []struct {
		name   string
		start  int
		count  int
		limit  int
		seed   int
		asJSON bool
		pretty bool
	}{
		{"version alone", 0, defaultCount, 0, 0, false, false},
		{"version with -json -n 5", 0, 5, 0, 0, true, false},
		{"version with -pretty", 0, defaultCount, 0, 0, false, true},
		{"version with invalid -n 0", 0, 0, 0, 0, false, false},
		{"version with -start 10", 10, 5, 0, 0, false, false},
		{"version with invalid -start -1", -1, 5, 0, 0, false, false},
		{"version with -limit 3", 0, defaultCount, 3, 0, false, false},
		{"version with invalid -limit -1", 0, 5, -1, 0, false, false},
		{"version with -seed 10", 0, 5, 0, 10, false, false},
		{"version with invalid -seed -1", 0, 5, 0, -1, false, false},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var buf bytes.Buffer
			if err := run(&buf, opts(tt.start, tt.count, tt.limit, tt.seed, "", tt.asJSON, tt.pretty, true, false)); err != nil {
				t.Fatalf("run(version) returned error: %v", err)
			}
			if want := "gofib " + Version + "\n"; buf.String() != want {
				t.Errorf("run(version) wrote %q, want exactly %q", buf.String(), want)
			}
		})
	}
}

// TestRunLimit pins the -limit semantics: a positive limit caps the
// largest index printed (0, the default, is the "no limit" sentinel,
// mirroring -start's 0), the output is the intersection of -start,
// -n, and -limit, and a limit below start yields empty output and
// success — not an error.
func TestRunLimit(t *testing.T) {
	tests := []struct {
		name      string
		start     int
		n         int
		limit     int
		asJSON    bool
		wantLines int
		wantFirst string
		wantLast  string
	}{
		// limit 0 (unset or explicit) means no limit at all.
		{"limit 0 means no limit", 0, 5, 0, false, 5, "1: 1", "5: 5"},
		// A huge -n is capped by the limit and finishes immediately:
		// no index above the limit is ever computed or printed.
		{"limit caps a huge -n", 0, 100000, 10, false, 10, "1: 1", "10: 55"},
		// A limit at or above the range changes nothing.
		{"limit above the range changes nothing", 0, 5, 50, false, 5, "1: 1", "5: 5"},
		{"limit equal to the last index changes nothing", 0, 5, 5, false, 5, "1: 1", "5: 5"},
		// Intersection with -start: start <= index <= min(start+n-1, limit).
		{"limit and start intersect", 5, 10, 7, false, 3, "5: 5", "7: 13"},
		// limit < start (both positive): zero lines, exit 0.
		{"limit below start prints nothing", 10, 5, 3, false, 0, "", ""},
		// -json on the reduced set: one object per surviving index.
		{"limit with -json", 0, 5, 2, true, 2, wantJSONLine(1), wantJSONLine(2)},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var buf bytes.Buffer
			if err := run(&buf, opts(tt.start, tt.n, tt.limit, 0, "", tt.asJSON, false, false, false)); err != nil {
				t.Fatalf("run(%d, %d, %d) returned error: %v", tt.start, tt.n, tt.limit, err)
			}
			if tt.wantLines == 0 {
				if buf.Len() != 0 {
					t.Fatalf("run(%d, %d, %d) wrote %q, want empty output", tt.start, tt.n, tt.limit, buf.String())
				}
				return
			}
			lines := strings.Split(strings.TrimSuffix(buf.String(), "\n"), "\n")
			if len(lines) != tt.wantLines {
				t.Fatalf("run(%d, %d, %d) printed %d lines, want %d", tt.start, tt.n, tt.limit, len(lines), tt.wantLines)
			}
			if lines[0] != tt.wantFirst {
				t.Errorf("first line = %q, want %q", lines[0], tt.wantFirst)
			}
			if lines[len(lines)-1] != tt.wantLast {
				t.Errorf("last line = %q, want %q", lines[len(lines)-1], tt.wantLast)
			}
		})
	}

	// -pretty on the reduced set: the columns are sized from the
	// limit-capped last index (here 10 and Fib(10) = 55), not from
	// start+n-1 (which would be 12 and Fib(12) = 144).
	// F(8)..F(10); hardcoding F-values is allowed only for small n.
	prettyFibs := map[int]string{8: "21", 9: "34", 10: "55"}
	t.Run("limit with -pretty sizes columns from the capped last index", func(t *testing.T) {
		const start, n, limit = 8, 5, 10
		var buf bytes.Buffer
		if err := run(&buf, opts(start, n, limit, 0, "", false, true, false, false)); err != nil {
			t.Fatalf("run(%d, %d, %d, pretty) returned error: %v", start, n, limit, err)
		}
		lines := strings.Split(strings.TrimSuffix(buf.String(), "\n"), "\n")
		if len(lines) != 3 {
			t.Fatalf("run(%d, %d, %d, pretty) printed %d lines, want 3", start, n, limit, len(lines))
		}
		for i, line := range lines {
			idx := start + i
			// Widths come from the capped last index (10) and its
			// value (Fib(10) = 55), not from start+n-1.
			if want := prettyLine(limit, idx, prettyFibs[idx]); line != want {
				t.Errorf("line %d = %q, want %q", i+1, line, want)
			}
		}
	})
}

// TestRunRejectsInvalidLimit pins the -limit validation contract: a
// negative limit exits non-zero with the exact -start-style error
// message (parseOptions is the rejection point).
func TestRunRejectsInvalidLimit(t *testing.T) {
	for _, mode := range []struct {
		name   string
		asJSON bool
	}{{"text", false}, {"json", true}} {
		for _, limit := range []int{-1, -5} {
			args := []string{"-limit", strconv.Itoa(limit)}
			if mode.asJSON {
				args = append(args, "-json")
			}
			_, err := parseOptions(args)
			if err == nil {
				t.Fatalf("parseOptions(%d, %s) succeeded, want error", limit, mode.name)
			}
			want := fmt.Sprintf("invalid value %d for flag -limit: must be >= 0", limit)
			if err.Error() != want {
				t.Errorf("parseOptions(%d, %s) error = %q, want exactly %q", limit, mode.name, err.Error(), want)
			}
		}
	}
}

// TestRunSeed pins the -seed semantics: a positive seed prints exactly
// one entry for that index in the active output mode, overriding
// -n, -start, and -limit (whose validation is skipped, mirroring
// -version's ignore-invalid semantics); seed 0 is the unset sentinel,
// so plain gofib output is unchanged.
func TestRunSeed(t *testing.T) {
	tests := []struct {
		name      string
		seed      int
		start     int
		n         int
		limit     int
		asJSON    bool
		pretty    bool
		wantLines int
		wantFirst string
		wantLast  string
	}{
		// Single index lookup in each output mode.
		{"seed 10 prints only index 10", 10, 0, defaultCount, 0, false, false, 1, "10: 55", "10: 55"},
		{"seed 1 prints only index 1", 1, 0, defaultCount, 0, false, false, 1, "1: 1", "1: 1"},
		{"seed 100 prints F(100)", 100, 0, defaultCount, 0, false, false, 1,
			"100: " + mustBig("354224848179261915075").String(),
			"100: " + mustBig("354224848179261915075").String()},
		// Precedence: -seed overrides every range flag.
		{"seed overrides -n", 10, 0, 5, 0, false, false, 1, "10: 55", "10: 55"},
		{"seed overrides -start", 10, 3, defaultCount, 0, false, false, 1, "10: 55", "10: 55"},
		{"seed overrides -limit", 10, 0, defaultCount, 5, false, false, 1, "10: 55", "10: 55"},
		{"seed overrides -n, -start, and -limit together", 12, 2, 5, 7, false, false, 1, "12: 144", "12: 144"},
		// Range-flag validation is skipped when seed > 0 (mirrors
		// -version's ignore-invalid semantics).
		{"seed skips invalid -n 0", 10, 0, 0, 0, false, false, 1, "10: 55", "10: 55"},
		{"seed skips invalid -start -1", 10, -1, 5, 0, false, false, 1, "10: 55", "10: 55"},
		{"seed skips invalid -limit -1", 10, 0, 5, -1, false, false, 1, "10: 55", "10: 55"},
		// Sentinel: seed 0 behaves like unset — range flags rule.
		{"seed 0 is the unset sentinel", 0, 0, 5, 0, false, false, 5, "1: 1", "5: 5"},
		{"seed 0 with -start keeps the range", 0, 10, 2, 0, false, false, 2, "10: 55", "11: 89"},
		// Output-mode combos: one JSON object / one pretty row.
		{"seed with -json", 10, 0, defaultCount, 0, true, false, 1, wantJSONLine(10), wantJSONLine(10)},
		{"seed with -pretty sizes columns from the sole index", 10, 0, defaultCount, 0, false, true, 1, prettyLine(10, 10, "55"), prettyLine(10, 10, "55")},
		{"seed with -json and -pretty is plain JSON", 10, 0, defaultCount, 0, true, true, 1, wantJSONLine(10), wantJSONLine(10)},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var buf bytes.Buffer
			if err := run(&buf, opts(tt.start, tt.n, tt.limit, tt.seed, "", tt.asJSON, tt.pretty, false, false)); err != nil {
				t.Fatalf("run(seed=%d) returned error: %v", tt.seed, err)
			}
			lines := strings.Split(strings.TrimSuffix(buf.String(), "\n"), "\n")
			if len(lines) != tt.wantLines {
				t.Fatalf("run(seed=%d) printed %d lines, want %d", tt.seed, len(lines), tt.wantLines)
			}
			if lines[0] != tt.wantFirst {
				t.Errorf("first line = %q, want %q", lines[0], tt.wantFirst)
			}
			if lines[len(lines)-1] != tt.wantLast {
				t.Errorf("last line = %q, want %q", lines[len(lines)-1], tt.wantLast)
			}
		})
	}
}

// TestRunRejectsInvalidSeed pins the -seed validation contract: a
// negative seed exits non-zero with the exact -start/-limit-style
// error message. It fires even when the range flags would otherwise
// error, because -seed is validated before them (parseOptions is the
// rejection point).
func TestRunRejectsInvalidSeed(t *testing.T) {
	for _, mode := range []struct {
		name   string
		asJSON bool
	}{{"text", false}, {"json", true}} {
		for _, seed := range []int{-1, -5} {
			args := []string{"-seed", strconv.Itoa(seed)}
			if mode.asJSON {
				args = append(args, "-json")
			}
			_, err := parseOptions(args)
			if err == nil {
				t.Fatalf("parseOptions(seed=%d, %s) succeeded, want error", seed, mode.name)
			}
			want := fmt.Sprintf("invalid value %d for flag -seed: must be >= 0", seed)
			if err.Error() != want {
				t.Errorf("parseOptions(seed=%d, %s) error = %q, want exactly %q", seed, mode.name, err.Error(), want)
			}
		}
	}
}

// TestRunFormatModes pins each -format mode's exact output for the
// same range (indices 8..12): text ("<index>: <value>"), json (one
// object per line), pretty (both columns right-aligned), and table
// (value only, right-justified to the widest value).
func TestRunFormatModes(t *testing.T) {
	for _, tt := range []struct {
		name   string
		format string
		want   string
	}{
		{"text", "text", "8: 21\n9: 34\n10: 55\n11: 89\n12: 144\n"},
		{"json", "json",
			wantJSONLine(8) + "\n" + wantJSONLine(9) + "\n" + wantJSONLine(10) + "\n" + wantJSONLine(11) + "\n" + wantJSONLine(12) + "\n"},
		{"pretty", "pretty", " 8:  21\n 9:  34\n10:  55\n11:  89\n12: 144\n"},
		{"table", "table", " 21\n 34\n 55\n 89\n144\n"},
		{"csv", "csv", "8,21\n9,34\n10,55\n11,89\n12,144\n"},
		{"empty format is text (unset sentinel)", "", "8: 21\n9: 34\n10: 55\n11: 89\n12: 144\n"},
	} {
		t.Run(tt.name, func(t *testing.T) {
			var buf bytes.Buffer
			if err := run(&buf, opts(8, 5, 0, 0, tt.format, false, false, false, false)); err != nil {
				t.Fatalf("run(format=%q) returned error: %v", tt.format, err)
			}
			if buf.String() != tt.want {
				t.Errorf("run(format=%q) = %q, want %q", tt.format, buf.String(), tt.want)
			}
		})
	}
}

// TestRunFormatShortcutEquivalence pins that -json and -pretty are
// pure shortcuts: their output is byte-identical to the corresponding
// -format mode, in both the range path and -seed lookup mode.
func TestRunFormatShortcutEquivalence(t *testing.T) {
	for _, tt := range []struct {
		name   string
		format string
		asJSON bool
		pretty bool
		seed   int
	}{
		{"-json equals -format json (range)", "json", true, false, 0},
		{"-pretty equals -format pretty (range)", "pretty", false, true, 0},
		{"-json equals -format json (seed)", "json", true, false, 10},
		{"-pretty equals -format pretty (seed)", "pretty", false, true, 10},
		{"agreeing -json -format json is accepted", "json", true, false, 0},
		{"agreeing -pretty -format pretty is accepted", "pretty", false, true, 0},
	} {
		t.Run(tt.name, func(t *testing.T) {
			var viaFormat, viaShortcut bytes.Buffer
			n := defaultCount
			if err := run(&viaFormat, opts(0, n, 0, tt.seed, tt.format, false, false, false, false)); err != nil {
				t.Fatalf("run(format=%q) returned error: %v", tt.format, err)
			}
			if err := run(&viaShortcut, opts(0, n, 0, tt.seed, "", tt.asJSON, tt.pretty, false, false)); err != nil {
				t.Fatalf("run(shortcut) returned error: %v", err)
			}
			if viaFormat.String() != viaShortcut.String() {
				t.Errorf("shortcut output %q != -format output %q", viaShortcut.String(), viaFormat.String())
			}
		})
	}
	// csv has no shortcut flag: -format csv is its only spelling, and
	// neither -json nor -pretty may stand in for it (conflicts are
	// pinned in TestRunFormatConflicts).
	t.Run("csv has no shortcut: -format csv alone is its only spelling", func(t *testing.T) {
		var buf bytes.Buffer
		if err := run(&buf, opts(0, 3, 0, 0, "csv", false, false, false, false)); err != nil {
			t.Fatalf("run(format=csv) returned error: %v", err)
		}
		if want := "1,1\n2,1\n3,2\n"; buf.String() != want {
			t.Errorf("run(format=csv) = %q, want %q", buf.String(), want)
		}
	})
}

// TestRunFormatConflicts pins the conflict rule: when -format is
// explicitly set, any also-given shortcut must agree. Both shortcut
// directions, plus shortcuts vs text and table, exit non-zero with a
// stderr error naming both flags and write no output. With -format
// unset there is no conflict check — legacy -json -pretty keeps its
// JSON-wins semantics (pinned by TestRunPrettyJSON).
func TestRunFormatConflicts(t *testing.T) {
	tests := []struct {
		name   string
		format string
		asJSON bool
		pretty bool
		want   string
	}{
		{"-pretty with -format json", "json", false, true,
			`flags -pretty and -format conflict: -pretty selects pretty but -format selects json`},
		{"-json with -format pretty", "pretty", true, false,
			`flags -json and -format conflict: -json selects json but -format selects pretty`},
		{"-json with -format text", "text", true, false,
			`flags -json and -format conflict: -json selects json but -format selects text`},
		{"-pretty with -format text", "text", false, true,
			`flags -pretty and -format conflict: -pretty selects pretty but -format selects text`},
		{"-json with -format table", "table", true, false,
			`flags -json and -format conflict: -json selects json but -format selects table`},
		{"-pretty with -format table", "table", false, true,
			`flags -pretty and -format conflict: -pretty selects pretty but -format selects table`},
		{"-json with -format csv", "csv", true, false,
			`flags -json and -format conflict: -json selects json but -format selects csv`},
		{"-pretty with -format csv", "csv", false, true,
			`flags -pretty and -format conflict: -pretty selects pretty but -format selects csv`},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var buf bytes.Buffer
			err := run(&buf, opts(0, 5, 0, 0, tt.format, tt.asJSON, tt.pretty, false, false))
			if err == nil {
				t.Fatalf("run(format=%q) succeeded, want conflict error", tt.format)
			}
			if err.Error() != tt.want {
				t.Errorf("error = %q, want exactly %q", err.Error(), tt.want)
			}
			if buf.Len() != 0 {
				t.Errorf("wrote %q before failing, want no output", buf.String())
			}
		})
	}
	// Legacy, no -format: -json -pretty is not an error (JSON wins).
	t.Run("legacy -json -pretty without -format is not a conflict", func(t *testing.T) {
		var buf bytes.Buffer
		if err := run(&buf, opts(0, 3, 0, 0, "", true, true, false, false)); err != nil {
			t.Fatalf("run(-json -pretty, no -format) returned error: %v", err)
		}
		if want := wantJSONLine(1) + "\n" + wantJSONLine(2) + "\n" + wantJSONLine(3) + "\n"; buf.String() != want {
			t.Errorf("output = %q, want %q", buf.String(), want)
		}
	})
}

// TestRunRejectsInvalidFormat pins the invalid-value contract: an
// unknown -format value exits non-zero with the flag-package-style
// error listing the valid modes and writes no output.
func TestRunRejectsInvalidFormat(t *testing.T) {
	for _, format := range []string{"xml", "TEXT", "jso"} {
		var buf bytes.Buffer
		err := run(&buf, opts(0, 5, 0, 0, format, false, false, false, false))
		if err == nil {
			t.Fatalf("run(format=%q) succeeded, want error", format)
		}
		want := fmt.Sprintf("invalid value %q for flag -format: must be one of text, json, pretty, table, csv", format)
		if err.Error() != want {
			t.Errorf("error = %q, want exactly %q", err.Error(), want)
		}
		if buf.Len() != 0 {
			t.Errorf("wrote %q before failing, want no output", buf.String())
		}
	}
}

// csvRecord returns the expected -format csv line for index i.
func csvRecord(i int) string {
	return fmt.Sprintf("%d,%s", i, Fib(i).String())
}

// TestRunCSV pins csv mode across the flag combinations: one plain
// "<index>,<fib>" record per line with no header, composing with -n,
// -start/-limit, -seed lookup (a single record), and -sum (exactly one
// "sum,<start>,<last>,<total>" record mirroring JSON's index_range+sum
// structure; an empty range sums to 0 with last < start).
func TestRunCSV(t *testing.T) {
	for _, tt := range []struct {
		name   string
		start  int
		n      int
		limit  int
		seed   int
		sum    bool
		wantEl int // expected line count (0 means empty output)
		first  string
		last   string
	}{
		{"csv default (n=3) prints three records", 0, 3, 0, 0, false, 3, csvRecord(1), csvRecord(3)},
		{"csv with -n 5 prints five records", 0, 5, 0, 0, false, 5, csvRecord(1), csvRecord(5)},
		{"csv with -start 10 -limit 12 prints indices 10..12", 10, 100, 12, 0, false, 3, csvRecord(10), csvRecord(12)},
		{"csv with -seed prints one record", 0, defaultCount, 0, 10, false, 1, csvRecord(10), csvRecord(10)},
		{"csv with -sum prints one sum record", 0, 10, 0, 0, true, 1, "sum,1,10,143", "sum,1,10,143"},
		{"csv with -sum and narrowing", 8, 5, 10, 0, true, 1, "sum,8,10,110", "sum,8,10,110"},
		{"csv with -sum over an empty range sums to 0", 5, 10, 3, 0, true, 1, "sum,5,3,0", "sum,5,3,0"},
	} {
		t.Run(tt.name, func(t *testing.T) {
			var buf bytes.Buffer
			if err := run(&buf, opts(tt.start, tt.n, tt.limit, tt.seed, "csv", false, false, false, tt.sum)); err != nil {
				t.Fatalf("run(csv) returned error: %v", err)
			}
			out := buf.String()
			if tt.wantEl == 0 {
				if out != "" {
					t.Fatalf("wrote %q, want empty output", out)
				}
				return
			}
			lines := strings.Split(strings.TrimSuffix(out, "\n"), "\n")
			if len(lines) != tt.wantEl {
				t.Fatalf("printed %d lines, want %d: %q", len(lines), tt.wantEl, out)
			}
			if lines[0] != tt.first {
				t.Errorf("first line = %q, want %q", lines[0], tt.first)
			}
			if lines[len(lines)-1] != tt.last {
				t.Errorf("last line = %q, want %q", lines[len(lines)-1], tt.last)
			}
		})
	}
}

// tableLine returns the expected -format table line for value v:
// right-justified to the width of the widest value printed, which is
// Fib(n) for the range ending at index n.
func tableLine(n int, v string) string {
	return fmt.Sprintf("%*s", len(Fib(n).String()), v)
}

// TestRunTableCombinations pins table mode composing with the range
// flags exactly like the other modes: -start shifts the window, -limit
// caps it (and the width follows the capped widest value), and a
// -seed lookup prints just the value.
func TestRunTableCombinations(t *testing.T) {
	for _, tt := range []struct {
		name      string
		start     int
		n         int
		limit     int
		seed      int
		wantLines int
		wantFirst string
		wantLast  string
	}{
		{"table with -start 8 -n 5", 8, 5, 0, 0, 5, tableLine(12, "21"), tableLine(12, "144")},
		{"table with -limit caps the window", 8, 5, 10, 0, 3, tableLine(10, "21"), tableLine(10, "55")},
		{"table with -limit below start is empty", 10, 5, 7, 0, 0, "", ""},
		{"table with -seed prints just the value", 0, defaultCount, 0, 12, 1, tableLine(12, "144"), tableLine(12, "144")},
		{"table with -seed 10", 0, defaultCount, 0, 10, 1, tableLine(10, "55"), tableLine(10, "55")},
	} {
		t.Run(tt.name, func(t *testing.T) {
			var buf bytes.Buffer
			if err := run(&buf, opts(tt.start, tt.n, tt.limit, tt.seed, "table", false, false, false, false)); err != nil {
				t.Fatalf("run(table) returned error: %v", err)
			}
			if tt.wantLines == 0 {
				if buf.Len() != 0 {
					t.Fatalf("wrote %q, want empty output", buf.String())
				}
				return
			}
			lines := strings.Split(strings.TrimSuffix(buf.String(), "\n"), "\n")
			if len(lines) != tt.wantLines {
				t.Fatalf("printed %d lines, want %d", len(lines), tt.wantLines)
			}
			if lines[0] != tt.wantFirst {
				t.Errorf("first line = %q, want %q", lines[0], tt.wantFirst)
			}
			if lines[len(lines)-1] != tt.wantLast {
				t.Errorf("last line = %q, want %q", lines[len(lines)-1], tt.wantLast)
			}
		})
	}
}

// TestRunFormatSeed pins that the active -format mode applies in -seed
// lookup mode too: -format json emits one object, -format table emits
// just the value.
func TestRunFormatSeed(t *testing.T) {
	t.Run("-seed with -format json equals -seed -json", func(t *testing.T) {
		var viaFormat, viaShortcut bytes.Buffer
		if err := run(&viaFormat, opts(0, defaultCount, 0, 10, "json", false, false, false, false)); err != nil {
			t.Fatalf("run(-seed -format json) returned error: %v", err)
		}
		if err := run(&viaShortcut, opts(0, defaultCount, 0, 10, "", true, false, false, false)); err != nil {
			t.Fatalf("run(-seed -json) returned error: %v", err)
		}
		if viaFormat.String() != viaShortcut.String() {
			t.Errorf("output %q != %q", viaFormat.String(), viaShortcut.String())
		}
		if want := wantJSONLine(10) + "\n"; viaFormat.String() != want {
			t.Errorf("output = %q, want %q", viaFormat.String(), want)
		}
	})
	t.Run("-seed with -format table prints just the value", func(t *testing.T) {
		var buf bytes.Buffer
		if err := run(&buf, opts(0, defaultCount, 0, 10, "table", false, false, false, false)); err != nil {
			t.Fatalf("run(-seed -format table) returned error: %v", err)
		}
		if buf.String() != "55\n" {
			t.Errorf("output = %q, want %q", buf.String(), "55\n")
		}
	})
}

// TestRunVersionFormat pins -version's precedence over -format: even
// an invalid mode value or a shortcut/format conflict is ignored —
// the only output is the version line, exit 0.
func TestRunVersionFormat(t *testing.T) {
	for _, tt := range []struct {
		name   string
		format string
		asJSON bool
		pretty bool
	}{
		{"version with invalid -format xml", "xml", false, false},
		{"version with -pretty -format json conflict", "json", false, true},
		{"version with -json -format pretty conflict", "pretty", true, false},
		{"version with -format table", "table", false, false},
	} {
		t.Run(tt.name, func(t *testing.T) {
			var buf bytes.Buffer
			if err := run(&buf, opts(0, 5, 0, 0, tt.format, tt.asJSON, tt.pretty, true, false)); err != nil {
				t.Fatalf("run(version) returned error: %v", err)
			}
			if want := "gofib " + Version + "\n"; buf.String() != want {
				t.Errorf("wrote %q, want exactly %q", buf.String(), want)
			}
		})
	}
}

// wantSumJSON returns the canonical JSON object for -sum mode with the
// effective bounds [first, last] and the big.Int sum as a string.
func wantSumJSON(first, last int) string {
	total := new(big.Int)
	for i := first; i <= last; i++ {
		total.Add(total, Fib(i))
	}
	b, err := json.Marshal(sumLine{IndexRange: []int{first, last}, Sum: total.String()})
	if err != nil {
		panic("marshal sumLine: " + err.Error())
	}
	return string(b)
}

// TestRunSum pins the -sum semantics: exactly one line carrying the
// big.Int sum of the same -start/-limit/-n intersection line mode
// would print, rendered per output mode. An empty selected range sums
// to 0 without error, index_range still reports the computed effective
// bounds even when inverted, a positive -seed is rejected, and
// -version keeps its precedence.
func TestRunSum(t *testing.T) {
	// fibSum returns the total of Fib(first)..Fib(last) as a string
	// (the sum can exceed int64, e.g. for the default 100 range).
	fibSum := func(first, last int) string {
		total := new(big.Int)
		for i := first; i <= last; i++ {
			total.Add(total, Fib(i))
		}
		return total.String()
	}
	for _, tt := range []struct {
		name   string
		start  int
		n      int
		limit  int
		format string
		asJSON bool
		pretty bool
		want   string
	}{
		// Sum of F(1)..F(10) = 143.
		{"default range in text mode", 0, 10, 0, "", false, false, "sum: 143\n"},
		{"-start narrows the range", 8, 5, 0, "", false, false, "sum: " + fibSum(8, 12) + "\n"},
		{"-start and -limit narrow the range", 8, 5, 10, "", false, false, "sum: " + fibSum(8, 10) + "\n"},
		{"-limit caps the range", 0, 100000, 5, "", false, false, "sum: 12\n"},
		{"empty range (-limit below -start) sums to 0", 10, 5, 3, "", false, false, "sum: 0\n"},
		{"empty range with -start 5 -limit 3", 5, 10, 3, "", false, false, "sum: 0\n"},
		{"json mode", 0, 10, 0, "json", false, false, wantSumJSON(1, 10) + "\n"},
		{"json mode with narrowing", 8, 5, 10, "json", false, false, wantSumJSON(8, 10) + "\n"},
		{"json mode with empty range reports inverted bounds", 5, 10, 3, "json", false, false, wantSumJSON(5, 3) + "\n"},
		{"json shortcut via -json", 0, 3, 0, "", true, false, wantSumJSON(1, 3) + "\n"},
		{"pretty mode prints the bare value", 0, 10, 0, "pretty", false, false, fibSum(1, 10) + "\n"},
		{"table mode prints the bare value", 0, 10, 0, "table", false, false, fibSum(1, 10) + "\n"},
		{"csv mode prints one sum record", 0, 10, 0, "csv", false, false, "sum,1,10," + fibSum(1, 10) + "\n"},
		{"csv mode with empty range", 5, 10, 3, "csv", false, false, "sum,5,3,0\n"},
		{"big default range sum exceeds int64 as text", 0, defaultCount, 0, "", false, false, "sum: " + fibSum(1, defaultCount) + "\n"},
	} {
		t.Run(tt.name, func(t *testing.T) {
			var buf bytes.Buffer
			if err := run(&buf, opts(tt.start, tt.n, tt.limit, 0, tt.format, tt.asJSON, tt.pretty, false, true)); err != nil {
				t.Fatalf("run(sum) returned error: %v", err)
			}
			if buf.String() != tt.want {
				t.Errorf("run(sum) = %q, want exactly %q", buf.String(), tt.want)
			}
		})
	}
}

// TestRunSumSeedConflict pins the -seed/-sum conflict contract: a
// positive -seed with -sum is an error naming both flags (rejected by
// parseOptions), while the unset sentinel (-seed 0) combines freely
// with -sum. A negative -seed keeps its existing error even when -sum
// is also set.
func TestRunSumSeedConflict(t *testing.T) {
	for _, seed := range []int{1, 10, 100} {
		_, err := parseOptions([]string{"-seed", strconv.Itoa(seed), "-sum"})
		if err == nil {
			t.Fatalf("parseOptions(seed=%d, sum) succeeded, want error", seed)
		}
		want := "flags -seed and -sum conflict: -seed prints a single index but -sum prints the range sum"
		if err.Error() != want {
			t.Errorf("parseOptions(seed=%d, sum) error = %q, want exactly %q", seed, err.Error(), want)
		}
	}
	// seed 0 is the unset sentinel: legal with -sum.
	t.Run("seed 0 with -sum is legal", func(t *testing.T) {
		var buf bytes.Buffer
		if err := run(&buf, opts(0, 5, 0, 0, "", false, false, false, true)); err != nil {
			t.Fatalf("run(seed=0, sum) returned error: %v", err)
		}
		if buf.String() != "sum: 12\n" {
			t.Errorf("run(seed=0, sum) = %q, want %q", buf.String(), "sum: 12\n")
		}
	})
	// A negative -seed keeps its existing error even alongside -sum.
	t.Run("negative seed keeps its error with -sum", func(t *testing.T) {
		_, err := parseOptions([]string{"-seed", "-1", "-sum"})
		if err == nil {
			t.Fatalf("parseOptions(seed=-1, sum) succeeded, want error")
		}
		if want := "invalid value -1 for flag -seed: must be >= 0"; err.Error() != want {
			t.Errorf("error = %q, want exactly %q", err.Error(), want)
		}
	})
}

// TestRunSumVersionPrecedence pins that -version outranks even the
// -seed/-sum conflict and the sum output itself.
func TestRunSumVersionPrecedence(t *testing.T) {
	for _, tt := range []struct {
		name string
		seed int
	}{
		{"version with -sum", 0},
		{"version with -sum and a conflicting -seed", 10},
	} {
		t.Run(tt.name, func(t *testing.T) {
			var buf bytes.Buffer
			if err := run(&buf, opts(0, 5, 0, tt.seed, "", false, false, true, true)); err != nil {
				t.Fatalf("run(version, sum) returned error: %v", err)
			}
			if want := "gofib " + Version + "\n"; buf.String() != want {
				t.Errorf("run(version, sum) wrote %q, want exactly %q", buf.String(), want)
			}
		})
	}
}

// TestParseOptionsDefaults pins every flag's default value: parsing no
// args yields n=100, start=0, limit=0, seed=0, format="", and all
// booleans false.
func TestParseOptionsDefaults(t *testing.T) {
	got, err := parseOptions(nil)
	if err != nil {
		t.Fatalf("parseOptions(nil) returned error: %v", err)
	}
	want := options{n: 100, start: 0, limit: 0, seed: 0, format: "", json: false, pretty: false, version: false, sum: false}
	if got != want {
		t.Errorf("parseOptions(nil) = %+v, want %+v", got, want)
	}
}

// TestParseOptionsInvalid covers every rejection parseOptions can
// make, each with its exact byte-identical error message.
func TestParseOptionsInvalid(t *testing.T) {
	for _, tt := range []struct {
		name string
		args []string
		want string
	}{
		{"n=0", []string{"-n", "0"},
			"invalid value 0 for flag -n: must be >= 1"},
		{"n=-1", []string{"-n", "-1"},
			"invalid value -1 for flag -n: must be >= 1"},
		{"start<0", []string{"-start", "-3"},
			"invalid value -3 for flag -start: must be >= 0"},
		{"limit<0", []string{"-limit", "-2"},
			"invalid value -2 for flag -limit: must be >= 0"},
		{"seed<0", []string{"-seed", "-1"},
			"invalid value -1 for flag -seed: must be >= 0"},
		{"seed+sum", []string{"-seed", "5", "-sum"},
			"flags -seed and -sum conflict: -seed prints a single index but -sum prints the range sum"},
		{"bad format value", []string{"-format", "xml"},
			`invalid value "xml" for flag -format: must be one of text, json, pretty, table, csv`},
		{"format json with -pretty", []string{"-format", "json", "-pretty"},
			"flags -pretty and -format conflict: -pretty selects pretty but -format selects json"},
		{"format pretty with -json", []string{"-format", "pretty", "-json"},
			"flags -json and -format conflict: -json selects json but -format selects pretty"},
	} {
		t.Run(tt.name, func(t *testing.T) {
			_, err := parseOptions(tt.args)
			if err == nil {
				t.Fatalf("parseOptions(%v) succeeded, want error", tt.args)
			}
			if err.Error() != tt.want {
				t.Errorf("parseOptions(%v) error = %q, want exactly %q", tt.args, err.Error(), tt.want)
			}
		})
	}
}

// TestParseOptionsVersionShortCircuit pins that -version is honored
// before any validation: parseOptions returns the version flag with no
// error even alongside flags that would otherwise be rejected.
func TestParseOptionsVersionShortCircuit(t *testing.T) {
	for _, tt := range []struct {
		name string
		args []string
	}{
		{"version with invalid -n", []string{"-version", "-n", "0"}},
		{"version with bad format", []string{"-version", "-format", "xml"}},
		{"version with shortcut conflict", []string{"-version", "-format", "json", "-pretty"}},
		{"version with negative seed", []string{"-version", "-seed", "-1"}},
		{"version with seed+sum conflict", []string{"-version", "-seed", "5", "-sum"}},
	} {
		t.Run(tt.name, func(t *testing.T) {
			opts, err := parseOptions(tt.args)
			if err != nil {
				t.Fatalf("parseOptions(%v) returned error: %v", tt.args, err)
			}
			if !opts.version {
				t.Errorf("parseOptions(%v) version = false, want true", tt.args)
			}
		})
	}
}
