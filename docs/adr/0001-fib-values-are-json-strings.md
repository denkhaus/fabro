# Fib values are JSON strings, not numbers

JSON numbers cannot carry F(79) and beyond exactly: most consumers parse
them as IEEE 754 doubles, losing precision above 2^53. The CLI's JSON mode
therefore emits fib as strings ("fib": "354224848179261915075"), keeping
values lossless without assuming a big-number-aware consumer. Index stays a
number (always < 2^53). Changing this breaks the output contract of every
machine consumer; reopen only for a major version.
