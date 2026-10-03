# Improve review — run 01M2YVNCFP5RDFCFRW9FS42R0B

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (63.2 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-20 08:56+0000 by revisor `fabro_ask`

---

All evidence is in hand — stage timings/costs and journal entries from run events/checkpoints, seed statuses verified against the tracker file at `.seeds/issues.jsonl`. Run baseline for impact ranking: 62.6 min wall, $4.81 LLM cost; implementer@1 alone was 50.8 min / $4.18 (87% of cost); the only gate red and one full extra cycle came from a pre-existing test defect, not the seed work.

## Recommendations, ordered by expected impact

**1. Kill the stale-build/stale-binary class in the implementer lane — extend fabro-56db with an arm: normalize mtimes at sandbox/verify start, not only at gate start.**
What happened: implementer@1's journal (from run journal diff) records "Stale build fingerprints forced two blind rebuild cycles: an E0425 … and an E0063 … plain `cargo build -p <dependent>` used cached artifacts and reported green"; implementer@2 then hit "nextest reused a stale test binary after edit_file twice … nearly misdiagnosing a correct fix as wrong." All of this sits inside the 87%-of-cost implementer stage. fabro-56db (open) already prescribes the `find lib -name '*.rs' ! -newermt 2000-01-01 -exec touch {} +` one-liner, but only in `scripts/qualitygate.nu`. Concrete change: add an arm moving/applying it to `scripts/verify.nu` (or sandbox init) so it covers the implementer's own verify calls. Expected effect: no blind rebuild cycles, no stale-binary misdiagnosis; zero LLM cost.

**2. Audit the workspace for root-blind `#[cfg(unix)]` permission-based tests — new seed.**
What happened: the sole gate failure (from tester@1 event: "2681 tests run: 2680 passed, 1 failed") was pre-existing `publish_failure_before_flip_leaves_old_bundle_consistent` — chmod 0o000 is void under uid 0, so it fails deterministically in run containers while green on dev hosts. It cost a full extra cycle (tester@1 99.8 s + gatebounce + implementer@2 274 s / $0.21 + tester@2 52.3 s ≈ 7.1 min) and consumed one of three gate cycles toward deadlock. The implementer's journal explicitly asks for "an audit for other `#[cfg(unix)]` permission-based tests" (naming `mirror_dist_preserves_unchanged_assets` as metadata-only, unaffected). New-seed justification: grep of the tracker shows no seed covers the root-blind chmod-injection class (the in-run fix mx-6f6c12 repaired one test only). Expected effect: eliminates pre-existing deterministic-red tests blocking seed cycles.

**3. Implement fabro-2b1b — drop the hand fmt/clippy triplet from briefs.**
What happened: this run's brief still prescribed `cargo +nightly-2026-04-14 fmt/clippy -p fabro-workflow` scoped to ONE crate, while the diff touched seven (gate output: "touched crates: fabro-llm, fabro-manifest, fabro-workflow, fabro-api, fabro-config, fabro-dev, fabro-types") and the mechanical `just verify implementer` re-derived the full set — the double-compile and wrong-scope-hand-commands pattern fabro-2b1b (open) was filed on. Concrete change: `.fabro/workflows/develop/prompts/planner.md` step 6/7 and `implementer.md` step 4, per the seed body. Expected effect: one fewer compile pass per implementer stage and no under-scoped hand verification in briefs.

**4. Implement fabro-c841 — tighten gate-bounce matching to error signatures.**
What happened: on the gate red, `gate-bounce.nu` attached three known-bug hits (fabro-41de prompt-lint, fabro-f18a tool-JSON lint, fabro-71b9 kanban tiles — ~4 KB inline into implementer@2's context) none of which had any relation to the actual spa_refresh/chmod failure. Label-level matching produced pure noise on the one bounce this run. Concrete change: `.fabro/workflows/develop/scripts/gate-bounce.nu`, require token overlap with error code/crate/file path, emit `{"hits":[]}` otherwise. Expected effect: bounce passes start at the real root cause instead of reading three irrelevant seed bodies.

**5. Implement fabro-8cef — emit the evidence blob as plain-text multi-line.**
What happened: the evidence capture was 63,685 bytes as a single JSON-escaped line (from the evidence stage event); the reviewer's journal painpoint: "read_file/grep return truncated copies of the whole line with no way to page the omitted middle — the only reliable fallback was reading the touched repo files directly." That fallback means the reviewer verified against the worktree, not the capture, weakening the evidence pipe. Concrete change: `.fabro/workflows/develop/scripts/evidence.nu` writes a multi-line blob so one `read_file` with offset/limit suffices. Expected effect: reviews stay on the capture; removes substring-paging misparse risk (~60–90 s per asset-heavy review, per the seed's basis).

**6. Implement fabro-db25 arm 1 — materiality bar on the closeout residual sweep.**
What happened: this run's closeout filed fabro-a60f from the reviewer's explicitly-labeled-"theoretical" nit (expired-deadline re-emit in `ttft.rs` `wrap()`), status open at priority 2 — exactly the self-perpetuating non-defect work generator fabro-db25 (open) arm 1 targets ("a recurring ~$0.50/7-min work generator per non-defect observation"). Concrete change: `closeout.nu` sweep files only findings naming a defect with a target; cosmetic/theoretical nits stay journaled or file at P4/cosmetic. Expected effect: reviewer footnotes stop spawning full dev cycles.

**7. Mark fabro-af22 and fabro-23d4 blocked-external — new seed (or direct user tracker action).**
What happened: planner@1 spent roughly six tool calls (~40 s of its 122 s stage; events seq 53–91: `sd show fabro-af22`, four git fetch/ls-remote/merge-base probes of upstream #784, `sd show fabro-23d4`) re-deriving that both top candidates are unclaimable, and journaled "sd ready keeps surfacing it every run — suggest the user marks it blocked-external or links a blocker." New-seed justification: no existing seed covers af22/23d4 (fabro-a285 covers fabro-d810 only); the user-side action needs no seed at all. Expected effect: those probes disappear from every subsequent planner pass.

**8. Require real descriptions on `ml record` at capture time — new seed.**
What happened: the run's lesson record mx-490000 (pattern `ttft-timeout-at-stream-middleware-seam`) landed with the literal description "verify id" (visible in the `.mulch/expertise/engine.jsonl` diff); the reviewer journaled it as a painpoint ("the pattern name is valuable but the body is empty"). New-seed justification: existing ml seeds cover when to record (fabro-ee2c), printing the mx-id (fabro-8d81), and flag docs (fabro-96bd) — none requires a non-placeholder description. Concrete change: one line in `implementer.md`'s lesson-capture section (and/or the record call) mandating a self-contained description. Expected effect: expertise records stay useful to `ml search` instead of starving.

One cross-cutting note: the seed this run implemented (fabro-e395, TTFT timeout) is itself the fix for the 163 s first-token stall class observed in run 01M2NVXCSQ5 — once PR #322 merges and the server/toolchain deploy picks it up, planner/implementer latency spikes of that class are bounded at 45 s. The deferred TS-client regen follow-up is already tracker-visible as fabro-82ec; no action lost.
