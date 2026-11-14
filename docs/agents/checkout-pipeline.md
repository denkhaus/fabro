# The run checkout pipeline

Who prepares what, from create to sandbox — the one-page map (grown from
the 2026-11-14 retry-on-lab archaeology: three crates, 30+ minutes of
grep).

```
create/retry/fork          engine (petri)              sandbox
     │                          │                         │
     ├─ platform worktree ──────┤                         │
     │  lib/apps/fabro-server/  │                         │
     │  server/petri_runs.rs    │                         │
     │  prepare_run_worktree()  │                         │
     │  → GitCheckout::prepare_worktree()                 │
     │    (git_checkout.rs: bare-clone cache +            │
     │     fetch selector + worktree add)                 │
     │  → /storage/scratch/<run>/worktree                 │
     │    at the target branch tip                        │
     │                          │                         │
     │  The run branch: fabro/run/<run-id> — created by   │
     │  the RUN's own checkpoint commits (pushed), or by  │
     │  a fork's seeding (fork.rs writes the RunBranch     │
     │  record; the engine's restore creates it).         │
     │                          │                         │
     ├──────────────────────────┤                         │
     │  checkout step (at start): pack_clone() packs the  │
     │  PLATFORM worktree into the sandbox                │
     │  vendor/petri/crates/attractor/steps/src/          │
     │  checkout.rs::seed()                                │
     │                          ├──→ sandbox /workspace   │
```

Key facts:

- The PLATFORM (fabro-server) prepares the worktree; the ENGINE packs
  it. Nothing clones inside the sandbox.
- A fresh run's worktree sits at the target branch tip; the run's
  checkpoint commits make it drift, and the run branch is pushed as
  `fabro/run/<run-id>` (the merger integrates these branches).
- A fork does NOT re-prepare: it seeds from the source's checkpoint
  (`fabro_petri::fork` — records, snapshots, run branch) and the
  successor's worktree is restored from that branch at start.
- retry-on-latest (work#1320) is the exception that re-prepares: the
  create path cuts the worktree at the CURRENT tip and
  `run_rebase.rs` cherry-picks the source run branch's commits on top.
- The toolchain environment is cooked at run init from the CHECKOUT
  (ADR-0042): the checkout's `.fabro/Dockerfile` (or the line's
  `Dockerfile.toolchain`) is the build context. A workflow DEFINES
  environments (`[environments.<id>]`); the create request SELECTS one
  (`--environment` / the automation body's `environment:` field) —
  declaring without selecting falls to the default sandbox (#1319).

Cook cost budget (user directive 2026-11-14 — plan with it): the cook
adds 0–30 minutes to the FIRST lap at a new checkout state — 0 when
every layer is a cache hit, ~10 minutes when only the compile layer
rebuilds (deps stay cached via the chef pattern), up to 30 minutes
cold (observed specimen: run 01M7D3E048F94C7X21Q9HYPAT4 on
2026-11-14). With the line merging PRs hourly, nearly every lap is the
first at its tip. The line graphs' stall timeouts carry this —
develop 63m (comfortable), merger 46m (tight with merge work),
adjudicator 31m (at the ceiling: a cold cook plus any stall kills the
lap) — until #1335 replaces the CLI compile with the pinned release
binary and collapses the cook to layer hits + downloads.
