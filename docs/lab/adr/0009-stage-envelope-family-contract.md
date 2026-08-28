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
