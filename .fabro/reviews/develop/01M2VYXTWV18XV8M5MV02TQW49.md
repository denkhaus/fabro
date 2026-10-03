# Improve review — run 01M2VYXTWV18XV8M5MV02TQW49

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (16.4 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-19 04:51+0000 by revisor `fabro_ask`

---

# Run 01M2VYXTWV18XV8M5MV02TQW49 (fabro-ead4) — improvement recommendations

Baseline from run events: 16.6 min wall, **$1.43** total. Planner 202 s / $0.46 (32%), implementer 683 s / $0.90 (63%), reviewer 41 s / $0.066, gate 7.9 s ("no crates touched"). Zero retries, one clean cycle — the costs below are all structural, not failures.

---

**1. Mechanically skip upstream-gated seeds in the preflight (graph design + error handling)**
Evidence (run events seq 44–99 + planner journal): the planner burned ~60 s and several LLM rounds re-adjudicating `fabro-af22` — git-log greps, `find_skill_references` searches, a failed `origin/main` probe — only to conclude "upstream-gated, skip," which its own painpoint says happens **every lap**.
Change: add an arm to `.fabro/workflows/develop/scripts/planner-preflight.nu` that parses a `UPSTREAM PR OPEN: <repo>#<n>` line from candidate bodies, checks whether the fix commit is an ancestor of `origin/denkhaus`, and marks the row `upstream_gated: true` (mirroring the existing `in_flight` marking), so the planner skips it mechanically.
Effect: removes ~60–90 s and ~$0.10–0.15 per develop lap, recurring.
**New-seed justification:** no open seed covers upstream-PR gating — fabro-af22 itself tracks the blocked work, and closest seed fabro-d9f7 (stale in_progress requeue) is a different mechanism.

**2. Top-N `sd ready` view instead of the firehose (tool efficiency)**
Evidence (seq 37–39): the planner's first call returned **200 seeds / 28.9 KB**, of which only the top 3 were adjudicated; conversation tokens grew 7.3k → 51k across the lap.
Change: `--limit 10` in the sd command table in `.fabro/workflows/develop/prompts/planner.md` PROJECT_FACTS (keep `--limit 200` for the rare full `sd list`).
Effect: ~30–40% fewer planner input tokens per lap, faster tie-breaking.
**Seed: fabro-c3b4** (existing, exact match; fabro-55a7/fabro-e4fa cover the complementary recon-batching).

**3. Expose `current_seed_id` in the `fabro_runs_list` projection (graph design)**
Evidence (seq 40–41): the in-flight tool call returned 73 runs — including the run itself and many terminal runs — all sharing the identical goal text, so it yielded nothing the planner could use; exclusion worked only because the preflight had already flagged fabro-c643.
Change: implement seed fabro-9372 in the engine projection (`lib/components/fabro-workflow/src/services.rs` area) plus the `created_since` bound from fabro-6b58.
Effect: in-flight exclusion becomes a table join instead of journal greps; one ~10 s tool call per lap becomes actually load-bearing.
**Seeds: fabro-9372, fabro-6b58** (both open, exact match).

**4. Vendored nushell skill for the implementer (prompting + error handling)**
Evidence (implementer journal + usage): implementer was 63% of run cost (22.3k reasoning tokens); 2 of its 3 observations were nu-0.115 pitfalls (mut-record type inference, interpolation parens) and it lost one full battery run to a `path self` fixture issue — the same semantics class as the latent crash it had to diagnose in pre-existing code.
Change: land `.fabro/skills/nushell-scripts/SKILL.md` (mut type inference, `$"…($id)…"` parens, `complete` vs try/catch, `path self` resolution) and reference it in `implementer.md`'s loop-asset carve-out, per the seed.
Effect: fewer re-derivation rounds and wasted battery runs on the frequent nushell-only seeds like this one.
**Seed: fabro-d19e** (existing, exact match).

**5. Cap the implementer prompt's step-4 Rust block (prompting)**
Evidence (stage prompt + usage): the implementer consumed 76.2k input tokens for a nushell-only change where the long step-4 Rust/gate policy was entirely irrelevant (gate printed "no crates touched").
Change: `implementer.md` step 4 — keep the operative rule, move run citations to footnotes per fabro-7b2a.
Effect: smaller preambles on every implementer pass, lower input cost and TTFT, especially on non-Rust seeds.
**Seed: fabro-7b2a** (existing, exact match).

**6. Fix the report-only "Already landed" route asymmetry (error handling)**
Evidence (reviewer journal, obs 3): in `--report-only` mode a top candidate with verdict `duplicate` and null sha still routes `Already landed` with `closed.seed: null` — the new ambiguity guard applies only to the live arm, and the implementer's real-env dry-run exercised exactly that mode.
Change: in `planner-preflight.nu`, gate the route expression on a resolvable sha for the top candidate in **both** live and report-only modes.
Effect: dry-run routing matches live semantics; prevents a dry-run-shaped exit without a close.
**New-seed justification:** fabro-ead4 closed on the live-arm guard; the residual report-only asymmetry it left behind (this run's reviewer observation) has no filed seed.

**7. Carry tool-result errors through the provider protocol (error handling)**
Evidence (planner session): four `unsupported_control` warnings ("this provider protocol does not support the tool result error flag") plus two shell probes flagged `is_error` for expected shapes — grep exit 1 (no-match) and `git log origin/main` exit 128 (the shallow clone only has `origin/denkhaus`).
Change: land fabro-b09c (protocol-level error flag) and add the fabro-7808 prompt line (grep exit 1 = expected no-match) plus a PROJECT_FACTS note that the run clone carries only `origin/denkhaus`.
Effect: probes stop masquerading as failures; no recovery rounds spent on non-errors.
**Seeds: fabro-b09c, fabro-7808** (both existing).

**8. Per-stage cost/painpoint digest in the PR body (user experience)**
Evidence: the $1.43 cost split (planner 32% / implementer 63%) and three journal observations are only visible by digging through run events; PR #262's body carries none of it.
Change: postlude change per fabro-1409 — append the per-stage cost/time table and journal painpoint digest to the PR body.
Effect: the reviewer/user sees where time and money went, and painpoints get a durable surface outside `.fabro/journal/`.
**Seed: fabro-1409** (existing, exact match).

---

Not recommended from this run: gate-speed work (gate was 7.9 s — the nushell-only fast path worked exactly as designed) and reviewer-prompt changes beyond #7's budget note — the reviewer approved in 41 s / $0.066 with zero painpoints, so fabro-cf3e's 16→32 KB inline raise is real but below these eight in impact.
