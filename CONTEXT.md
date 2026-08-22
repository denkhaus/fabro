# fabro lab — product world (denkhaus-lab)

This branch hosts gofib, a small Go CLI, built seed-by-seed by the develop
workflow. Platform decisions — the two-world architecture, the painpoint
channel, asset sync, the platform gate — live on `meta/denkhaus-lab`
(docs/adr there); its CONTEXT.md owns the dev-loop vocabulary (painpoint,
mailbox, refiner, platform/product seed). This glossary owns gofib.

## Language

**gofib**:
The product: a Go CLI printing Fibonacci numbers. Module `gofib`, package
main in `main.go`. _Avoid_: "the fibonacci app" in commit or seed titles.

**Fib**:
`Fib(n int) *big.Int` — the unit-testable computation, separate from output.
F(100) overflows int64, hence math/big. _Avoid_: recomputing inline.

**Text mode**:
Default output: one line per number, `<index>: <value>` (e.g. `100: 354224848179261915075`).

**JSON mode**:
`-json` output: JSON Lines, one object per number: `{"index":<int>,"fib":"<string>"}`.
_fib_ is a string by decision (ADR-0001). _Avoid_: emitting a JSON array.

**Count flag**:
`-n <int>` — how many numbers to print (default 100; >= 1 or non-zero exit
with stderr error). Combines with JSON mode.

**Quality gate**:
The opaque `just qualitygate` contract the implementer answers to. Treat as
green/black; its checks are the project's own business. _Avoid_: "tester".

## Related contexts

Platform vocabulary and architecture ADRs: `meta/denkhaus-lab`, its
CONTEXT.md and docs/adr/. Do not duplicate them here.
