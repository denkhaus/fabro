package main

import (
	"bytes"
	"encoding/json"
	"math/big"
	"strings"
	"testing"
)

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
			if err := run(&buf, tt.n, false); err != nil {
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
			if err := run(&buf, tt.n, true); err != nil {
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

func TestRunRejectsInvalidCount(t *testing.T) {
	for _, mode := range []struct {
		name   string
		asJSON bool
	}{{"text", false}, {"json", true}} {
		for _, n := range []int{0, -5} {
			var buf bytes.Buffer
			err := run(&buf, n, mode.asJSON)
			if err == nil {
				t.Fatalf("run(%d, %s) succeeded, want error", n, mode.name)
			}
			if !strings.Contains(err.Error(), "-n") {
				t.Errorf("run(%d, %s) error %q does not mention the -n flag", n, mode.name, err.Error())
			}
			if buf.Len() != 0 {
				t.Errorf("run(%d, %s) wrote %q before failing, want no output", n, mode.name, buf.String())
			}
		}
	}
}
