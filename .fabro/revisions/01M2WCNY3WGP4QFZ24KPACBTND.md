# Revision — run 01M2WCNY3WGP4QFZ24KPACBTND

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2WCNY3WGP4QFZ24KPACBTND.md
- seeds filed: none — zero filing credit this pass (0 stale/superseded closes); three consolidated findings survive dedupe and ride the journal as overflow
- balance: 0 non-exempt seeds filed / 0 — no credit this pass
- basis: run 01M2WCNY3WGP4QFZ24KPACBTND, workflow version 8b7bbe9d58b678ad56c49416d28afbf7872db2deec867d142043b12649cefcc8, commit 0b631568040768f0707bda0a8932f0b792495adf
- revised_at_commit: 0b631568040768f0707bda0a8932f0b792495adf (ADR-0015: engine drift signal for later judgement)

## Findings

### 1. Prescribe `just validate-workflows` for graph validation in develop prompts; forbid cargo build — OVERFLOW (not filed, no credit)
- overflow: file one seed — in `.fabro/workflows/develop/prompts/planner.md` step-6 cost-tier bullet and `implementer.md` step-4 whitelist, seeds touching `.fabro/workflows/**` must use `just validate-workflows <slug>` (justfile:196, wraps the toolchain-baked `fabro-validate` binary); a cargo build for graph validation is forbidden. Basis evidence: run 01M2WCNY's implementer spent most of 331.6s tool time building the CLI (665.4s stage, 67% of run cost) while the tester proved the diff in 8.2s with no crates touched. Expected effect: −4–6 min implementer tool time per loop-asset seed. Dedupe: closed fabro-513e/fabro-af97 built the tiers/recipe; no open seed prescribes the just recipe in prompts (fabro-4be6 is dry-running, fabro-6e7f is the test dispatcher). NEXT PASS: re-run dedupe, then file.

### 2+3. planner-preflight.nu multi-arm (consolidated same-file) — OVERFLOW (not filed, no credit)
- overflow: file ONE multi-arm seed (fabro-ae74 schema) for `.fabro/workflows/develop/scripts/planner-preflight.nu`:
  - Arm A — references_open_seed extraction + widen candidate table: extract `fabro-\w+` ids from each candidate body into a `references_open_seed: [...]` verdict-table column and widen the table beyond top-5 so the eventually-claimed seed is normally pre-covered. Basis: fabro-d9f7's body said "extend fabro-0da8's node" which did not exist; the planner burned ~6 LLM rounds / 5 probes (104.3s, $0.188) because d9f7 fell outside the top-5 table. Expected effect: dependency-shaped candidates resolve in one table read (fabro-9ec3 policy).
  - Arm B — replace `try { ... | from json } catch { null }` (lines 197/206/284) with the `describe`-based record type test already shipped in `tracker-guard.nu`'s `sd-issue-count`. Basis: nu's `from json` echoes non-JSON input back instead of erroring, so the try/catch silently misses shape changes (lesson mx-1b1551). Expected effect: removes a silent fail-open in the first node every run executes.
  - Dedupe: open preflight seeds (fabro-2afd fallback, fabro-e8ae anchors, fabro-ab93 in_flight, fabro-2ade mut-type bug) cover different changes; no overlap. NEXT PASS: re-run dedupe, then file.

### 4. Raise reviewer `preamble_inline_max_kb` from 16 — DUPLICATE, dropped
- duplicate_of: fabro-cf3e (open: "Develop reviewer node: raise preamble_inline_max_kb from 16 to 32" — same node, same attribute, same concrete change; the evidence here — 16,898-byte capture blob-ref'd, reviewer's only tool call a 27.4s `read_file` detour — strengthens fabro-cf3e and belongs there, not in a sibling seed).

### 5. Declare `output.planner` in the planner node's context_allow_keys — OVERFLOW (not filed, no credit)
- overflow: file one seed — add `output.planner` to the planner node's `context_allow_keys` in `.fabro/workflows/develop/workflow.fabro` (line 94 currently declares only `output.preflight`), or have the engine's response-dedup writer skip self-node drops. Basis: every planner pass emits the `context_allow_keys dropped: output.planner` warn (seq 97 this run); harmless (claim_check ok) but pure log noise that revisor WARN digests inherit. Expected effect: silences a per-pass warn without behavior change. Cross-ref: open fabro-7028 fixes the same mismatch class for the gatebounce node — thematic overlap only, NOT supersession (different node); cross-reference fabro-7028 in the new seed's description when filed. NEXT PASS: re-run dedupe, then file.

## Journal

- painpoints: ADR-0022 zero-credit pass journals three actionable consolidated findings as overflow; without a next-pass stale/superseded close they will keep overflowing — consider whether the overflow ledger itself needs a drain path (e.g. allow one carry-over filing per pass) or these will never land.
- observations: fabro-cf3e (open, reviewer preamble_inline_max_kb) should absorb this run's blob-detour evidence (16,898-byte capture, 27.4s read_file detour) — noted here for the human gate since the revisor does not edit user seeds.
