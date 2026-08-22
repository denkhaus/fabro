package main

import (
	"bytes"
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
			if err := run(&buf, tt.n); err != nil {
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

func TestRunRejectsInvalidCount(t *testing.T) {
	for _, n := range []int{0, -5} {
		var buf bytes.Buffer
		err := run(&buf, n)
		if err == nil {
			t.Fatalf("run(%d) succeeded, want error", n)
		}
		if !strings.Contains(err.Error(), "-n") {
			t.Errorf("run(%d) error %q does not mention the -n flag", n, err.Error())
		}
		if buf.Len() != 0 {
			t.Errorf("run(%d) wrote %q before failing, want no output", n, buf.String())
		}
	}
}
