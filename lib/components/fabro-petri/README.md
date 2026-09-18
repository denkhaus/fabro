# fabro-petri

Fabro's adapters over Petri, the workflow engine Fabro runs its workflows on.

## Layering rule

Only this crate imports Petri. The workspace `Cargo.toml` pins the Petri
packages by revision under `petri_*` keys, and `fabro-petri` is the only
member that lists them as dependencies. Every other Fabro crate reaches the
engine through what this crate exports. A Petri pin move is therefore a change
to this crate and the lockfile, nothing else.

## What it holds

Every adapter the integration plan describes lands here.

- `SqliteRunStore`: Petri's `RunStore` and `RunLogs` over Fabro's SQLite
  database, so a run's records live in Fabro's tables (`petri_runs` for the
  run and its writer lease, `petri_records` for every record of every log,
  and the shared `blobs` table). The module docs state the lease and append
  rules.
- `HttpRunStore`: the same store as a run's worker process reaches it, over
  the server's `/api/v1/runs/{id}/petri/*` endpoints with the worker's token.
  The server answers from its `SqliteRunStore`, so the lease and the
  `(log, seq)` rule are the store's; this layer carries requests, resends a
  request whose reply was lost, and maps the server's error codes back to
  `StoreError`. The module docs state the rules.
- `petri`: the Petri store vocabulary re-exported for the server, which
  answers the worker endpoints from a `SqliteRunStore` without naming a Petri
  package in its own manifest.
- The platform adapters the plan adds after it: hooks, interviews, secrets,
  output storage, run tools, the event projection.

## How it is tested

Integration tests live under `tests/`:

- `runs.rs` runs the `hello` bundle in memory through `Runtime::standard()`
  with the Fabro frontend and the model-free stub registry, then a
  command-only workflow on the host sandbox through the real step registry.
  Both skip, and say why, when the `sandbox-driver-host` plugin executable
  is not on `PATH` (every run takes its scope's environment through it);
  the sandbox-plugins CI job requires them.
- `sqlite_store.rs` runs Petri's store conformance suite
  (`petri_testkit::run_store::conformance`) against `SqliteRunStore`, plus the
  operator release, lease exclusivity, a crash between appends, and blob
  interoperation with Fabro's `BlobStore`.

The conformance suite over `HttpRunStore` needs a server to talk to, so it
lives with the server's integration tests
(`lib/apps/fabro-server/tests/it/api/petri_store.rs`), which reach the suite
through this crate's `test-support` feature (`fabro_petri::test_support`).

Run them with:

```sh
ulimit -n 4096 && cargo nextest run -p fabro-petri
```
