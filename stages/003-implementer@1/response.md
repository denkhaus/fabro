Done. Summary of this pass:

**Implemented seed fabro-e6df** — `README.md` at the repo root documenting the gofib CLI:

- **Intro**: gofib is a Go CLI printing Fibonacci numbers, module `gofib` — CONTEXT.md vocabulary used verbatim (text mode, JSON mode, count flag).
- **Install/build**: `mise install`, `just bootstrap`, gate `just qualitygate`.
- **Usage**: all example blocks are **captured, not invented** — I built the binary into `/tmp/gofib-readme` (outside the worktree, per the recorded artifact-hygiene failure mx-a58327) and mechanically verified every block against the live binary afterwards: 10/10 exact matches (text mode, JSON mode incl. the F(100) string, `-pretty` alignment, `-version`, `-n 0` stderr + exit 1, version precedence over `-n 0 -json`, `-json -pretty` no-effect).
- **Flag reference**: all four flags with the required annotations — `-n` default 100 / `>= 1` / combines with JSON mode; `-json` JSON Lines, `fib` string per ADR-0001, never an array; `-pretty` aligned columns **with explicit "no effect with `-json`" interaction documented**; `-version` prints exactly `gofib 1.3.0` from the `Version` const and takes precedence over every other flag.

**Hygiene**: temp build dir removed; worktree shows only `?? README.md` (no binaries — and `gofib` was already gitignored). Smoke check `go test ./...` green; full `just qualitygate` left to the tester step as required.

No new durable insight surfaced (the build-outside-worktree convention was already recorded and merely followed), so no mulch record was added.

{
  "outcome": "succeeded",
  "preferred_next_label": "Implemented",
  "context_updates": {
    "implementation_summary": "Created `/workspace/fabro/README.md` documenting the gofib CLI: intro (Go CLI printing Fibonacci numbers, module `gofib`), install/build via `mise install`/`just bootstrap` with `just qualitygate` as the gate, usage examples, and a four-flag reference (`-n` count flag default 100 with >= 1 validation, `-json` JSON Lines with string fib per ADR-0001, `-pretty` aligned columns with the explicit no-effect-with-`-json` interaction, `-version` printing exactly `gofib 1.3.0` with precedence over all other flags). All example output was captured from a real build in `/tmp/gofib-readme` (outside the worktree) and mechanically verified 10/10 against the binary; temp dir removed, `go test ./...` green, worktree clean except README.md."
  }
}