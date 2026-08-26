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
