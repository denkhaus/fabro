Done! Here's the wrap-up:

## 👋 Hello!

This session itself was the demo — I was executing the `hello` workflow from `.fabro/workflows/hello/workflow.fabro`:

| Stage | What happened |
|---|---|
| **Start** | Workflow engine admitted the run, goal: *"Say hello and demonstrate a basic Fabro workflow"* |
| **Greet** | Agent stage (me) received the prompt *"Add a haiku to the README"* and did the work |
| **Exit** | Run completes with this summary |

That's the simplest possible Fabro graph — `start -> greet -> exit` — the same shape as the [hello-world tutorial](https://docs.fabro.sh/tutorials/hello-world), just with real tools doing real file edits along the way.

## 🌸 The haiku

Added to the end of `README.md`:

> Graphs define the work,
> agents turn each node to code.
> Humans gate, then rest.

(5-7-5, and it happens to summarize the whole product: workflows-as-graphs, agent execution, human gates.)

**Files changed:** `README.md` (+8 lines). No commit made — the workflow engine's checkpointing owns that if this run is being tracked by the server.