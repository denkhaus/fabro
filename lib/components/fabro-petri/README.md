# fabro-petri

Fabro's adapters over Petri, the workflow engine Fabro runs its workflows on.

## Layering rule

Only this crate imports Petri. The workspace `Cargo.toml` pins the Petri
packages by revision under `petri_*` keys, and `fabro-petri` is the only
member that lists them as dependencies. Every other Fabro crate reaches the
engine through what this crate exports. A Petri pin move is therefore a change
to this crate and the lockfile, nothing else.

## What it holds

Every adapter the integration plan describes lands here: the run store over
Fabro's SQLite database, then the platform adapters (hooks, interviews,
secrets, output storage, run tools, the event projection).

## How it is tested

Integration tests live under `tests/`:

- `runs.rs` runs the `hello` bundle in memory through `Runtime::standard()`
  with the Fabro frontend and the model-free stub registry, then a
  command-only workflow on the host sandbox through the real step registry.
  The sandbox test skips, and says why, when the `sandbox-driver-host` plugin
  executable is not on `PATH`.

Run them with:

```sh
ulimit -n 4096 && cargo nextest run -p fabro-petri
```
