// Command gofib prints the first 100 Fibonacci numbers, one per line,
// prefixed with the index: "1: 1", "2: 1", "3: 2", ...
package main

import (
	"fmt"
	"math/big"
)

// Fib returns the n-th Fibonacci number, with F(1) = F(2) = 1.
func Fib(n int) *big.Int {
	a, b := big.NewInt(0), big.NewInt(1)
	for i := 0; i < n; i++ {
		a, b = b, new(big.Int).Add(a, b)
	}
	return a
}

func main() {
	for i := 1; i <= 100; i++ {
		fmt.Printf("%d: %v\n", i, Fib(i))
	}
}
