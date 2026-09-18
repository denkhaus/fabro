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
- `check`: Petri compiles at create time. The workflow version's bundle is
  materialized into a temporary directory (`Runtime::check` reads files from
  disk), lowered with the run's inputs and launch, and the admitted graphs or
  Petri's diagnostics come back in a shape the server maps onto Fabro's.
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
- `projection`: the fold of a Petri run's public events (`replay_since` over
  its records) and Fabro's platform records (`fabro-store`'s
  `platform_records`) into the `RunProjection` the API serves, row by row as
  `VIEWS.md` maps them. The stage key is `(execution, firing)`; the
  `StageId` label is `node@visit`, made unique with the execution when two
  child invocations would share one.
- `projector`: the view pass and its wake-up. Records first: Petri's append
  and a platform record's insert return before any view work; a pass reads
  what is committed, folds the items past the committed positions, and
  writes the projection document (`petri_projection`), the ordered stream
  (`petri_stream`, one `stream_seq` per Petri event or platform record) and
  the narrowed `runs` row in one later transaction. The server signals the
  projector after each committed worker append, after each committed
  platform record (the run summary store's hook), at worker exit and, over
  every Petri run, at startup. A run that executes in the server process
  goes through `Projector::observe_store`, which signals after each append.
  A torn tail (a record Petri cannot read) holds the view where it stands
  and reports the run incomplete with the reason.
- The platform adapters the plan adds after it: hooks, interviews over
  Fabro's API, secrets, output storage, run tools.

### What the projection leaves default

`VIEWS.md` rows with no source yet, or whose source this crate does not read
yet, keep their default value in the projection: `StageProjection.diff` and
`Conclusion.diff.patch` (the checkpoint's `patch_blob` is not resolved),
`Checkpoint`'s engine-derived maps (`completed_nodes`, `node_retries`,
`context_values`, `node_outcomes`, `next_node_id`), `agent_tools`,
`permission_level`, `script_invocation` and `script_timing`, a stage's
`notes`, `StageCompletion` details for a `parsed.note`, the sandbox instance
(the matrix's two gaps), `Run.ask_fabro`, an interview option's
`description` and `preview`, the pull request `creation` state, and the
run's notices, notifications and pairings (recorded, not shown).

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
  the blob store, binds the launch, and refuses an unknown attribute and,
  with a model client over the test catalog, an unknown model
  (`attractor.model.unknown`). No plugin is needed.
- `sqlite_store.rs` runs Petri's store conformance suite
  (`petri_testkit::run_store::conformance`) against `SqliteRunStore`, plus the
  operator release, lease exclusivity, a crash between appends, and blob
  interoperation with Fabro's `BlobStore`.

- `projection.rs` builds the view live (every append signals the
  projector) for the `hello` bundle on the stub registry, a command-only
  workflow and a two-branch parallel workflow, and checks it equals the view
  rebuilt from the records alone (`projector::rebuild`); catches a view up
  after every wake-up was dropped, by a signal and by the startup pass;
  recovers a crash between the record commit and the view transaction by
  applying only the missing suffix, with the positions and `stream_seq`
  continuing; runs two projectors over one store with child executions; and
  holds the view at a torn tail. All skip without the host plugin.

The conformance suite over `HttpRunStore` needs a server to talk to, so it
lives with the server's integration tests
(`lib/apps/fabro-server/tests/it/api/petri_store.rs`), which reach the suite
through this crate's `test-support` feature (`fabro_petri::test_support`).

Run them with:

```sh
ulimit -n 4096 && cargo nextest run -p fabro-petri
```

The server's end-to-end coverage is `lib/apps/fabro-server/tests/it/scenario/petri.rs`:
the `hello` bundle on the OpenAI twin, a command-only bundle and a
two-branch parallel bundle run to completion through the create handler and
the scheduler, in the server process under its test override, under the
version flag and under the server setting, with `GET /runs/{id}/state`
serving the projection over Petri's records, and Petri's diagnostics refuse
a run at create. The
server's `petri_runs` unit tests cover the lease ending at worker exit and
the restart reconcile that relaunches a worker in resume mode.

The worker path is covered with the real binary in
`lib/apps/fabro-cli/tests/it/scenario/petri.rs`: a command-only Petri run
executes in the worker a foreground server launched, its records reach
`petri_records` over the HTTP store and its lease ends with the worker; and
a run whose server and worker are both killed mid-stage resumes in a new
worker after the server restarts, with one `run.completed`.
