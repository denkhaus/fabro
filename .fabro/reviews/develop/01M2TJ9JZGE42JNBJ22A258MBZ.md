# Improve review — run 01M2TJ9JZGE42JNBJ22A258MBZ

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (71.5 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-18 17:12+0000 by revisor `fabro_ask`

---

All evidence below is from this run's events/checkpoints (stage timings, costs, journal records) and the tracker at `.seeds/issues.jsonl` (read via workspace file tools).

**Run baseline (for impact ranking):** wall 71.5 min, total cost $6.24. The implementer stage alone was 62 min / $6.02 (96.5% of cost; 149 shell + 43 edit_file calls, 33 min of pure tool time — mostly cold Rust builds). Planner 43 s/$0.08, reviewer 87 s/$0.14, gate 372 s. First-pass approval, zero retries — so all recommendations target friction the run *worked through anyway*.

## Recommendations, by expected impact

**1. Execute the approved toolchain-image bake — fabro-c643 (open, P1, user GO 2026-09-17).**
Change: rebuild `.fabro/Dockerfile.toolchain` per fabro-c643's four consolidated arms (cargo-chef layers, cargo-insta, graphviz, mold linker); alternatively mount a persistent sccache volume via `run.environment` in `.fabro/workflows/develop/workflow.fabro`.
Evidence: implementer tool_time 1,992,971 ms in a fresh `lifecycle.preserve=false` container; gate log shows it even rebuilding the renderer binary ("building fabro CLI renderer binary") inside the tester.
Effect: the seed's own estimate is 4–7 min off every run; this run's tool-time profile suggests more. Largest single wall/cost lever, already user-approved and just pending execution.

**2. Ship the stale-mtime mitigation and re-open the transport question — fabro-56db (open).**
Change: implement fabro-56db's touch (`find lib -name '*.rs' ! -newermt 2000-01-01 -exec touch {} +`) at the start of `scripts/verify.nu` (the implementer's entrypoint — the trap bites *before* the gate), mirroring it in `scripts/qualitygate.nu`.
Evidence: implementer journal painpoint 1 — file-tool edits left epoch (1970-01-01) mtimes, cargo/nextest silently ran stale binaries producing "logically impossible" failures, ~4 wasted diagnostic calls (lesson mx-6376dd). This is a *recurrence* of closed fabro-22e4, i.e. that fix is not holding on the current docker transport — worth noting in the seed body.
Effect: eliminates a silent wrong-verdict class (a stale binary can make a correct implementation "fail" or a broken one "pass") that has now struck at least two runs.

**3. Reopen fabro-3b1b (java in the run image) — and let closeout re-file the leftover — fabro-7aac (open).**
Change: reopen fabro-3b1b (its close reason says "reopen when in-run API regen becomes a real seed demand") with this run as the triggering evidence; separately implement fabro-7aac's closeout extension so the disclosed "re-run `bun run generate` on a Java-capable host" follow-up becomes a tracker seed instead of dying inside `implementation_summary`.
Evidence: `java: not found` blocked the seed's OpenAPI-first TS-regen acceptance criterion (painpoint 2, mx-7b4b22); the implementer hand-mirrored the two-line generator diff as a *disclosed deviation* the reviewer then had to adjudicate; closeout ran `sd close` in 0.4 s and filed nothing.
Effect: every API-schema seed currently ships a hand-mirrored generated-file diff — the single biggest review-trust and drift risk visible in this run.

**4. Fix verify.nu's inline-test blindness — fabro-2f70 (open).**
Change: in `scripts/verify.nu`, mark a crate test-file-touched when the diff touches `#[cfg(test)]` regions, not only `tests/` paths.
Evidence: implementer observation 3 — verify classified fabro-types/workflow/server as compile-only (tests live in inline modules), so the implementer manually ran the full suites (523+1542+970 tests) to compensate, exactly the fabro-2f70/fabro-6e7f failure mode.
Effect: the pre-gate test signal fires mechanically; removes the manual-compensation spend and the risk that a lazier pass ships untested code.

**5. Classify `docs/public/api-reference/` as seed work in evidence.nu — new seed.**
Change: add `docs/public/api-reference/**` to the seed-work path set in `.fabro/workflows/develop/scripts/evidence.nu`.
Evidence: reviewer journal painpoint — the spec-mandated OpenAPI edit was filed under "loop churn" and appeared in the mandatory-adjudication "changed files NOT named by the seed spec" section.
Effect: OpenAPI-first seeds stop forcing per-file manual adjudication and stop flirting with false "residue" rejections.
New-seed justification: closed fabro-2992 covers only the `.fabro/**`-as-loop-churn arm (by design); no open seed covers the docs/-path classifier arm.

**6. Raise the reviewer's inline ceiling so the capture stops blob-ref'ing — fabro-meta-c9f2 (open, same class).**
Change: in `.fabro/workflows/develop/workflow.fabro`, reviewer node `preamble_inline_max_kb=16` → ~56 (the attribute shipped via closed fabro-9467; graph budget is already 48 per closed fabro-1e9f — the per-node 16 KB is now the binding constraint for a 49.6 KB per-seed capture).
Evidence: this run's evidence capture was 49.6 KB; the reviewer got a blob ref and had to shell-page the whole blob after `read_file` truncated mid-diff (reviewer observation 1) — the fourth-plus occurrence of the fabro-meta-c9f2 class.
Effect: one fewer tool round-trip per review and removal of the truncated-read failure mode that previously caused false "Verification blocked" cycles.

**7. Correct fabro-c643's rotted anchor before the next claim — action on fabro-c643 itself.**
Change: one `sd update fabro-c643 --description "<full corrected body>"` fixing the `.fabro/Dockerfile.toolchai` citation (flagged `missing_file` by this run's preflight; the file the seed actually needs is `.fabro/Dockerfile.toolchain`).
Evidence: planner journal observation 2 + preflight verdict table (`anchors_ok: false`).
Effect: the next planner claims a corrected spec instead of re-adjudicating the flag — prevents the fabro-05d0 wrong-path re-probe class (~6 wasted calls / 39% of a run when it bites).

**8. Fix the style-guide first-action instruction to match the fs_hide envelope — fabro-81b7 (open, owns the guide-loading contract).**
Change: in `.fabro/workflows/develop/prompts/implementer.md` ("Rust work — read the vendored style guide FIRST"), replace "read … with `read_file`" with "read via shell (`cat`/`sed -n`) — `fs_hide` hides `.fabro/**` from file tools".
Evidence: run events seq 89–100 — the prescribed first `read_file` was denied by fs_hide, emitting a provider `unsupported_control` warning, before the shell fallback worked. Every Rust run pays this.
Effect: removes a deterministically-failing "hard gate" first action (1–2 wasted calls + warning noise per run) and stops teaching the agent that the prompt's hard rules can fail.

**Not recommended despite visibility:** the planner-stage `context_update_dropped: output.planner` warning (seq 61) is benign engine bookkeeping (the payload correctly lands in `response.planner`), and this run's planner/reviewer economics (43 s/$0.08 and 87 s/$0.14) leave nothing worth tuning there — the implementer stage and the sandbox environment are where this run's time and money went.
