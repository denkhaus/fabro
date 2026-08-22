package main

import (
	"math/big"
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
