# fabro-workflow

Fabro's platform half of a workflow run: what Fabro does around the engine.

Petri compiles and executes every run. `fabro-petri` is the crate that talks
to the engine (`fabro-dot` reads a graph's shape and file references through
Petri's parser), and this crate keeps what Fabro itself owns:

- **`operations`** — creating a run around Petri's admission
  (`materialize_admitted_run`, `persist_create_run`), and the other run
  operations: fork, rewind, retry, and the timeline they resolve targets on.
  The run's display graph (`fabro_types::RunGraph`) is read off the graph
  Petri admitted; the DOT the workflow was written in is persisted beside it
  as `graph_source`.
- **`workflow_bundle`** — the bundle a run is created from: every workflow
  of the version closure with its settings file and its files, and the
  `RunDefinition` the run records.
- **`git`**, **`sandbox_git`** — the Git helpers a run's platform effects use,
  on the host and inside a sandbox.
- **`pull_request`** — pull request creation for a finished run.
- **`run_tools`**, **`services`** — the run tools an agent session calls.
- **`web_search`** — the built-in web search backend.
- **`run_lookup`** — resolving a run selector to a run.

The run records and status vocabulary are `fabro_types`'. Workflow
diagnostics are Petri's: `fabro validate`, `fabro preflight` and the create
handler report Petri's codes (`attractor.*`, `unsupported.*`, `deprecated.*`,
`info.*`), plus Fabro's `fabro.model.no_ready_provider` when a model node has
no provider ready to run it.
