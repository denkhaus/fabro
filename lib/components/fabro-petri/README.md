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
- `runtime`: the Petri runtime Fabro assembles, the same way at create time
  and at execution: the Fabro frontend with the server's settings layer, the
  Attractor step kinds (real, or simulated for a dry run), the model client
  as the `PebbleClient` capability, the Fabro home.
- `check`: Petri compiles at create time. The workflow version's bundle goes
  into an in-memory file map (`frontend::MapFiles`, laid out as the bundle:
  `workflow.toml` beside the workflow, `.fabro/project.toml` at the root),
  `Runtime::check_source` lowers it with the run's inputs and launch, and the
  admitted graphs or Petri's diagnostics come back in a shape the server maps
  onto Fabro's. Nothing is written to disk.
- `admission`: the admitted graphs in Fabro's blob store, named on the run
  spec as `RunEngine::Petri(PetriAdmission)`, verified by digest on load.
- `engine`: a run executed by Petri, started from its admitted graphs or
  resumed from its records, with the outcome read from the run's record
  through `inspect_run` and mapped to the conclusion Fabro's read side
  records. The run's worker process runs it over `HttpRunStore`; the server
  runs it in its own process only under its test override, over
  `SqliteRunStore`. `interviewer::Unattended` fails any question until the
  interview adapter lands.
- `HttpRunStore`: the same store as a run's worker process reaches it, over
  the server's `/api/v1/runs/{id}/petri/*` endpoints with the worker's token.
  The server answers from its `SqliteRunStore`, so the lease and the
  `(log, seq)` rule are the store's; this layer carries requests, resends a
  request whose reply was lost, maps the server's error codes back to
  `StoreError`, and, for a worker, takes every lease for the worker's launch
  id. The module docs state the rules.
- `petri`: the Petri store vocabulary re-exported for the server, which
  answers the worker endpoints from a `SqliteRunStore` without naming a Petri
  package in its own manifest.
- The platform adapters the plan adds after it: hooks, interviews over
  Fabro's API, secrets, output storage, run tools, the event projection.

A run goes to Petri when its workflow version's `workflow.toml` names
`engine = "petri"` in `[workflow]`, or when the server's
`[server.execution] engine` (`FABRO_SERVER_ENGINE`, `fabro server start
--engine`) says so for versions that name none. The server side of both
halves is `fabro-server`'s `server::petri_runs`; the worker side is
`fabro-cli`'s `commands::run::petri_worker`, which `fabro run __run-worker`
takes when the run's stored spec names Petri. After a server restart, a
Petri run left in flight goes back to a worker in `--mode resume`: the run
continues from its records, as Petri's own resume does, and full recovery
of the workspace to a durable snapshot is the plan's F3.5.

## How it is tested

Integration tests live under `tests/`:

- `runs.rs` runs the `hello` bundle in memory through `Runtime::standard()`
  with the Fabro frontend and the model-free stub registry, then a
  command-only workflow on the host sandbox through the real step registry.
  Both skip, and say why, when the `sandbox-driver-host` plugin executable
  is not on `PATH` (every run takes its scope's environment through it);
  the sandbox-plugins CI job requires them.
- `check.rs` admits the `hello` bundle and round-trips its graph through
  the blob store, binds the launch, admits a version whose `workflow.toml`
  names `engine = "petri"`, reads the project settings from the map, and
  refuses an unknown attribute, an unknown `[workflow]` key
  (`unsupported.workflow_toml.key`, named in `workflow.toml`) and, with a
  model client over the test catalog, an unknown model
  (`attractor.model.unknown`). No plugin is needed.
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

The server's end-to-end coverage is `lib/apps/fabro-server/tests/it/scenario/petri.rs`:
the `hello` bundle on the OpenAI twin and a command-only bundle run to
completion through the create handler and the scheduler, in the server
process under its test override, under the version flag and under the
server setting, and Petri's diagnostics refuse a run at create. The
server's `petri_runs` unit tests cover the lease ending at worker exit and
the restart reconcile that relaunches a worker in resume mode.

The worker path is covered with the real binary in
`lib/apps/fabro-cli/tests/it/scenario/petri.rs`: a command-only Petri run
executes in the worker a foreground server launched, its records reach
`petri_records` over the HTTP store and its lease ends with the worker; and
a run whose server and worker are both killed mid-stage resumes in a new
worker after the server restarts, with one `run.completed`.
