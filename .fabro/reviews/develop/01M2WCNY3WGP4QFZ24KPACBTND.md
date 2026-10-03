# Improve review — run 01M2WCNY3WGP4QFZ24KPACBTND

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (14.3 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-19 08:54+0000 by revisor `fabro_ask`

---

All checks done. Grounding summary first, then the recommendations.

## What this run actually was (sources: run events, stage conclusion, journals)

Seed **fabro-0da8** (tracker guard node) claimed → implemented → gate green → approved → closed, PR #276. Total: **838.8s wall, $0.685**. Stage split (from stage conclusion): preflight 11.7s; **planner 104.3s / $0.188 (27% of cost)**; claim_check 0.09s; **implementer 665.4s / $0.461 (79% of wall, 67% of cost**, 331.6s tool time, 36 shell calls / 7 errors); tester 8.2s ("no crates touched … loop-asset scripts green"); evidence 0.3s; reviewer 27.4s / $0.037 (exactly 1 `read_file`); closeout 0.4s. Zero retries, one clean pass — the friction is all *cost shape*, not failures.

## Recommendations (by expected impact)

**1. Stop prescribing a cold `cargo build` to validate workflow graphs — the cheap remedy already exists.**
- What happened: the brief's own bullet prescribed "`cargo run -p fabro-cli -- parse`/`validate` … ONLY after a prior `cargo build`, timeout_ms ≥ 600000" (from planner context, seq 92); the implementer paid the bulk of its 331.6s tool time building the CLI to run `target/debug/fabro validate develop` (from implementation_summary: "built once, never cold cargo run"; fresh `target/debug/incremental/` artifacts confirm). The gate then proved the whole diff in **8.2s** with "no crates touched" — the build bought only the graph validation. Meanwhile `just validate-workflows [develop]` exists (from workspace file `justfile:196`) wrapping the toolchain-baked `fabro-validate` binary (`justfile:82–83`, seed af97) — seconds, zero build.
- Change: `.fabro/workflows/develop/prompts/planner.md` step-6 cost-tier bullet (and implementer.md step-4 whitelist): for seeds touching `.fabro/workflows/**`, the parse-tier check is `just validate-workflows <slug>`; a cargo build for graph validation is forbidden.
- Effect: −4–6 min and most of implementer tool time on every loop-asset seed — the tracker's dominant open class.
- Seed: **new-seed justification** — closed fabro-513e built the recipe, but no open seed teaches the planner/implementer to prescribe it (open fabro-4be6 is dry-running brief commands, a different mechanism).

**2. Preflight: flag candidates whose body references another OPEN seed (the d9f7→0da8 discovery chain).**
- What happened: fabro-d9f7's body says "extend fabro-0da8's node" — which didn't exist. The planner burned ~6 LLM rounds / 5 probes discovering this (seq 44–86: grep on the wrong file `workflow.toml`, `cat`, `sd show fabro-0da8`, `sd ready`, blocker check), then ran its own `git log --grep fabro-0da8` (seq 81–83) because the claimed seed fell **outside the preflight's top-5 verdict table** (af22, d9f7, a1ef, d950, fe6f only). Planner = 104s/$0.19 for a claim the preflight exists to mechanize; only planner diligence prevented implementing d9f7 against a nonexistent module.
- Change: `.fabro/workflows/develop/scripts/planner-preflight.nu` — extract `fabro-\w+` ids from each candidate body and add `references_open_seed: [...]` to the verdict table (fabro-9ec3 standing policy: mechanically-checkable invariants live in the preflight, never prose); widen the candidate table beyond top-5 so the eventually-claimed seed is normally pre-covered.
- Effect: dependency-shaped candidates resolve in one table read instead of ~6 rounds; the dup-check safety net covers the seed actually claimed.
- Seed: **new-seed justification** — searched dep/requeue/preflight/candidate: fabro-2afd (degraded fallback), fabro-4c81 (path anchors), closed fabro-ead4 (dup-close coverage) — none flags a candidate referencing another open seed.

**3. Bound `fabro_runs_list` — seed fabro-6b58 (open), this run is fresh regression evidence.**
- What happened: the planner's `fabro_runs_list {"workflow":"develop"}` took **10.54s** and returned **81 runs** (seq 37–38) — one call = 90% of the planner's 11.6s tool time; the current prompt text carries no `created_since` bound, exactly the regression fabro-6b58 documents (closed fabro-9967's bound was lost).
- Change: as specced verbatim in **fabro-6b58**: `planner.md` step 4 — pass `created_since ≈ 48h` plus explicit self-run exclusion.
- Effect: −10s and ~25KB of dead context on *every* planner lap, and the tax grows with the run catalog.

**4. Fix planner-preflight.nu's `from json` parse — latent failure in the first node every run executes.**
- What happened: this run's implementer observation + lesson mx-1b1551 + reviewer observation ("worth its own seed before a future sd output change trips it"): nu's `from json` echoes non-JSON input back instead of erroring, so the preflight's try/catch + null-check silently misses it. The correct pattern already shipped this run in `tracker-guard.nu`'s `sd-issue-count` (`describe`-based record type test).
- Change: `.fabro/workflows/develop/scripts/planner-preflight.nu` — replace the sd-parse try/catch with the `describe` type test.
- Effect: removes a silent-degradation class (fail-open today = the already-landed check quietly stops checking) from the node that gates every run's start.
- Seed: **new-seed justification** — open fabro-2ade is a different parse bug (mut type inference) in the same file; the reviewer verified this run that none covers the from-json/catch shape.

**5. Raise reviewer `preamble_inline_max_kb` 16 → 24.**
- What happened: the evidence capture was **17.1KB (16,898 bytes)** — over the 16KB node ceiling (`workflow.fabro:376`, from workspace file) — so it blob-ref'd, and the reviewer's *only* tool call of the pass was `read_file` to page it (27.4s stage). The graph's own comment for `preamble_budget_kb=48` (per fabro-1e9f) says per-seed captures (~18KB) "must render inline, zero blob detours" — the node attr contradicts that intent.
- Change: one number on the reviewer node in `.fabro/workflows/develop/workflow.fabro`.
- Effect: zero blob detours per review; eliminates the unread-blob → "Verification blocked" re-capture risk under fabro-269d's deliberately tool-poor reviewer.
- Seed: **new-seed justification** — fabro-9467 set 16 when captures were ~14KB, fabro-1e9f raised the graph budget (not this attr), open fabro-b568 is the opposite direction (inline floor for keys); no seed raises this ceiling.

**6. Land the ml-record flag contract — seeds fabro-96bd + fabro-f37b (open), paid again this run.**
- What happened: the implementer's first `ml record --type failure` failed validation on missing `--resolution` (journal painpoint, implementer@1; 1 of 7 errored shell calls).
- Change: exactly what **fabro-96bd** (implementer.md step-6 per-type flag table) and **fabro-f37b** (AGENTS.md full flag forms) already specify — this run is the Nth occurrence class.
- Effect: −1 failed call + retry round on every lesson-capturing pass. Rider, same class: the second painpoint (nu `return -1` parses as a flag; must be `return (-1)`) deserves a one-line convention note wherever loop-script rules live — **new-seed justification**: no seed teaches nu return conventions; mx records are pull-only.

**7. Silence the per-pass `context_allow_keys dropped: output.planner` warn.**
- What happened: every planner pass emits this warn (seq 97 this run) because the engine's response-dedup writes `output.planner` while the planner's own `context_allow_keys` doesn't declare it — harmless (all seed keys landed; claim_check ok), pure log noise that revisor WARN digests inherit.
- Change: declare the response-dedup key in the planner node's `context_allow_keys`, or have the dedup writer skip self-node drops — one line in `workflow.fabro`.
- Seed: **new-seed justification** — open fabro-7028 covers the same output_key/allow-keys mismatch class for the *gatebounce* node only.

Not inspected: the toolchain image contents themselves (baked `fabro-validate` verified only via the justfile comment) and the reviewer's materialized blob (path is engine-runtime, outside this workspace).
