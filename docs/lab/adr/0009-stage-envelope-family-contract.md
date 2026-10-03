# Stage envelope: one family contract, individually built attributes

The five per-node restriction seeds — preamble input (e47c), context
output (900e), tool access (47b5), filesystem scope (ba96), context pull
(e804; adjacent: skills d0d6) — are one design family, the **stage
envelope**: what a node sees, what it may write, which tools, files, and
skills it gets. Decided in the 2026-08-26 grilling session:

- **One contract, individual delivery.** Shared vocabulary, naming,
  default posture, and validation conventions; each attribute is built,
  tested, documented, and shipped on its own. No mega-attribute, no
  container syntax.
- **Default posture: open.** Restriction is always explicit per node.
  No graph-level strict mode in v1 — a default flip forces a whole-unit
  change across every world's prompts (ADR-0008) and buys nothing while
  both worlds are small.
- **Syntax: flat snake_case node attributes** with comma-list strings
  (`preamble_stages_ignore` precedent). `tools=` gets preset sugar
  (`tools=read` → read_file, grep, glob, list_dir). Dotted keys are
  foreign to the parser; node classes are rejected as a carrier for now
  — they can later become a render-time frontend over the flat attrs
  without engine changes.
- **Enforcement layer v1: tool/engine layer.** Render-only filters,
  dropped writes with stage events, `NamedToolAccessPolicy`, tool-file
  glob checks. Trust model is **drift protection, not adversarial
  containment**: shell remains a documented escape hatch (the sandbox
  binds nothing by design; the deployment is trusted single-tenant).
  Sandbox-level per-stage materialization only becomes a seed if an
  untrusted-payload world ever appears.
- **Build order by current pain:** 900e (context allowlist + append) →
  47b5 (tool policy; infrastructure exists) → e47c (re-evaluate: the
  a85b aggregate budget + deny-list may already cover the reviewer
  pain) → e804 (context_read; needs 47b5) → ba96 (largest tool surface).

The envelope is part of the foundations that let the develop workflow
run against the fabro repo itself; completion feeds the two-world
retirement tracked in seed fabro-a9bb (timing decided jointly).

## Amendment 2026-08-27 (fabro-47b5 design grilling)

The `tools=` preset sugar decided above is dropped before first
delivery: allow-only explicit comma lists. A preset name in the graph
hides the granted tool set, while an explicit list is self-documenting
and drift-safe when the catalog grows — new tools never leak into a
restricted node. Presets remain an additive later option if explicit
lists become unwieldy.

Also settled in the same session:

- No deny attribute and no run-level allowlist. Deny lists cannot
  express "read-only" safely (MCP tools depend on run config and
  cannot be enumerated in a graph; future native tools would leak),
  and a run-level list would reintroduce the two-layer override
  ambiguity (replace vs narrow) that allow-only removes. Consistent
  with the no-graph-level-strict-mode decision above.
- Question tools (`request_user_input`, `AskUserQuestion`) are exempt
  from the policy in both directions: HITL is a workflow contract, not
  model initiative (same posture as engine-stamped context keys in
  900e).
- `spawn_agent` requires explicit listing; child sessions inherit the
  node's allow-list, so the policy does not vanish at the child
  boundary.
- `permission_level` is untouched: it is upstream's interactive CLI
  approval axis (wire-contracted in the OpenAPI spec) and truthfully
  reports "no approval gating" (`Full`) for workflow stages. The
  per-stage tool-list projection (`agent_tools`) is the truthful
  policy reflection; no derived coarse label.
- Per-node `fabro_tools=` (opt-in list of `fabro_run_*` tools)
  overrides the run-wide `agent.fabro_tools` flag; unset keeps the run
  default. A default-denied opt-in family — the opposite posture of
  the default-open native registry.


## Amendment 2026-09-02 (fabro-e804 design grilling): the pull axis

`context_read` (e804) settles the fourth piece. Six grill decisions,
user-agreed:

1. **Name:** `context_read` (public contract, snake_case registry
   vocabulary).
2. **Posture:** default-registered in every agent stage — the family's
   default-open posture. Exclusion is the existing `tools=` allowlist;
   prompt nodes (tool-less) are unaffected.
3. **One window:** the readable set is non-engine keys ∩
   `preamble_allow_keys` (if set). This EXTENDS e47c's documented
   "render-only" semantics: the list is now the node's whole window on
   the context, inline AND on demand. Engine keys
   (`is_preamble_hidden_key` class incl. `response.*`, `outcome`) are
   never readable and cannot be re-admitted.
4. **Bounded results:** values over the stage's inline budget return the
   demote marker (preview+path); the tool reuses a85b's materialization
   and threshold resolution (node > graph > 8 KB) — no new knob.
5. **Key-only input in v1:** `read_file` owns files; path input is an
   additive follow-up only if `context_read`-only nodes must open
   demoted files.
6. **Deterministic view:** the served set is snapshotted from the node's
   resolved context at stage start (parallel-branch updates landing
   mid-run are not visible); a reused full-fidelity thread refreshes the
   view per node.

Implementation notes: `ContextReadServices` flows through the stage
request (resolved context + node envelope attrs + run-scoped run-store/
sandbox/run-dir); the registered executor closes over a shared,
per-node-refreshable state, so spawned subagent helpers serve the same
view. fabro-validate catalogs the name
(`KNOWN_WORKFLOW_TOOL_NAMES`, cross-checked against the workflow crate).

## Amendment 2026-09-03 (fabro-ba96 design grilling): the filesystem axis

`fs_hide`/`fs_write` settle the fifth and final piece. User-agreed decisions:

1. **Names:** flat `fs_hide` + `fs_write` — the dotted `fs.hide`/`fs.write`
   from the original proposal is rejected; dotted keys stay foreign to the
   parser (original family decision unchanged).
2. **Nonexistent semantics:** a hidden path behaves as if it did not exist —
   reads fail, `file_exists` is `false`, discovery results are filtered (no
   leak via `glob`/`grep`/`list_dir`), writes are denied. A `dir/**` glob
   also hides the `dir` entry itself.
3. **Write posture:** `fs_write` unset = open; set = allow-list only, with
   outside-workspace writes denied while a list is set; explicitly empty =
   read-only stage. Deletes and patch targets count as writes.
4. **Enforcement seam:** a `ScopedSandbox` decorator wraps the stage
   session's sandbox (and the subagent factory's), so every file tool in
   every vocabulary plus `apply_patch` paths are covered by one seam and
   children inherit the scope. Session-level reads (memory, skills) flow
   through the same window — feature, not bug. Run-level plumbing keeps
   its own unwrapped handle.
5. **Atomicity:** `apply_patch` pre-checks all target paths via the same
   policy (exposed through `ToolContext`) before applying anything.
6. **Validation:** invalid globs are errors; non-agent nodes warn (the
   47b5 precedent); `fs_write`-inside-`fs_hide` warns; shell-pairing is an
   Info note (drift-protection reminder), never a failure.

With ba96 the envelope family is complete; the two-world retirement
(fabro-a9bb) can draw on the full vocabulary.
