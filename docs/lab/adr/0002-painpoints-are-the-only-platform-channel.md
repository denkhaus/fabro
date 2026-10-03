# Stages never modify platform assets; they emit painpoints

During product runs, no stage may edit `.fabro/`, `scripts/`, `justfile`,
or `.mise.toml`. Observed friction is recorded as painpoints in
`context_updates.workflow_painpoints` (a flat context key, restated in full
by each stage that adds one) and delivered to the platform mailbox
(`painpoints.jsonl` on the meta branch) by a deterministic delivery step at
the end of the run. Platform seeds are then planned, implemented, gated and
reviewed by meta runs of the same develop workflow.

Known limitation (accepted): a flat context key is last-writer-wins; stages
must restate the accumulated list. The durable fix is a typed, append-only
routing field in the engine, proposed as an upstream issue alongside
seed fabro-af22 (skill expansion over preamble).
