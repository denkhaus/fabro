// Command gofib prints Fibonacci numbers, one per line, prefixed with
// the index: "1: 1", "2: 1", "3: 2", ... By default it prints the first
// 100; the -n flag changes how many are printed.
package main

import (
	"flag"
	"fmt"
	"io"
	"math/big"
	"os"
)

// defaultCount is how many numbers gofib prints when -n is omitted.
const defaultCount = 100

// Fib returns the n-th Fibonacci number, with F(1) = F(2) = 1.
func Fib(n int) *big.Int {
	a, b := big.NewInt(0), big.NewInt(1)
	for i := 0; i < n; i++ {
		a, b = b, new(big.Int).Add(a, b)
	}
	return a
}

// run writes the first count Fibonacci numbers to w as "<index>: <value>".
// It returns an error when count < 1.
func run(w io.Writer, count int) error {
	if count < 1 {
		return fmt.Errorf("invalid value %d for flag -n: must be >= 1", count)
	}
	for i := 1; i <= count; i++ {
		fmt.Fprintf(w, "%d: %v\n", i, Fib(i))
	}
	return nil
}

func main() {
	n := flag.Int("n", defaultCount, "how many Fibonacci numbers to print (must be >= 1; default 100)")
	flag.Parse()
	if err := run(os.Stdout, *n); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
