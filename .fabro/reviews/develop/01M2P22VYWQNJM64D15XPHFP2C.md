# Improve review — run 01M2P22VYWQNJM64D15XPHFP2C

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (39.96 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-16 22:36+0000 by revisor `fabro_ask`

---

## What happened in this run (basis for everything below)

Seed **fabro-3916** (agent-run attribution marker) went planner → implementer → **gate RED** → gatebounce → implementer@2 → gate green → evidence → reviewer (approved) → closeout. 40 min wall (21:31–22:11), **$1.72 LLM** (planner $0.12, implementer $1.54 across two visits, reviewer $0.06). The gate red was **2 of 2282 tests** — two inline `--help` snapshots drifted by the new `--agent-session` clap flag — and cost a full re-cycle: tester@1 326s + implementer@2 232s + gatebounce ≈ **9.3 min (~23% of run wall) + $0.15** to insert two one-line snapshots. (From run checkpoints and stage conclusions.)

## Recommendations, by expected impact

**1. Fix `verify.nu` inline-`#[cfg(test)]` detection — seed fabro-2f70 (open).**
Evidence: `scripts/verify.nu:24-25` classifies test files only as `/tests/` dirs, `_tests.rs`, `/tests.rs`. The implementer added a test mod inside `lib/apps/fabro-cli/src/args.rs`, so verify derived "test-file-touched=fabro-server only" (implementer@1 journal observation) and never ran the fabro-cli suite — the two failing help tests (`tests/it/cmd/{create,run}.rs`) first executed in the gate. One change: extend `is-test-file`/the classifier at verify.nu:68-81 so a diff touching `#[cfg(test)]` content in `src/**` marks the crate test-file-touched (full suite). Add the companion one-liner to `.fabro/workflows/develop/prompts/implementer.md` step 3: a clap `RunArgs` change requires updating the two inline help snapshots. Expected effect: this drift class dies pre-gate — on this run, ~9 min and one full implementer pass saved.

**2. Bake `cargo-insta` + a Java-free TS-client regen path into the toolchain image — new seed.**
Evidence: implementer@1 painpoint — `bun run generate` is impossible (openapi-generator needs Java; sandbox has none), so the generated TS client was **hand-edited at 4 `createRun` sites** in `runs-api.ts` (a disclosed deviation the reviewer had to adjudicate as "noted but not blocking"); implementer@2 painpoint — `cargo insta` absent, snapshot fix done from the nextest diff. Change: add both to `ghcr.io/denkhaus/fabro-toolchain` (image `99c855a689b3`). Expected effect: eliminates a recurring correctness-risk class and review overhead on every API-touching seed. New-seed justification: no open seed covers toolchain-image tooling gaps — fabro-fe15 is debug fabro-cli/cargo-cache only; the workarounds live only in mx-d7486b/mx-d955b8.

**3. Generalize the evidence anomaly exemption to any seed-named path — seed fabro-d76c (open).**
Evidence: reviewer painpoint — `evidence.nu` classified `docs/public/api-reference/fabro-api.yaml` as "loop churn" even though the brief's wire-contract bullet mandated that exact change, forcing manual anomaly adjudication. Change: in `.fabro/workflows/develop/scripts/evidence.nu`, exempt from the anomaly section any changed path the in-progress seed body names (d76c is scoped to fs_hide paths like `implementer.md`; extend it to `docs/public/api-reference/` and any named target). Expected effect: no per-API-seed manual adjudication, lower false-rejection risk.

**4. Tighten gate-bounce matching to error signatures — seed fabro-c841 (open).**
Evidence: gatebounce's 3.7KB output matched fabro-f18a/fabro-528b/fabro-12fe (conductor / `fabro_run_create` themes) against a help-snapshot tail, and the implementer prompt orders hits read *before* re-deriving root cause — pure misdirection on the bounce pass. Mechanism (from workspace file `gate-bounce.nu:25-28,129-136`): ≥2 distinct compound tokens from the tail vs any open `workflows`-labelled seed. Change: match on failure-signature tokens (nextest `FAIL` test names, panic/assert text) instead of generic tail tokens. Expected effect: bounce context shrinks to relevant hits (here: zero), cheaper and less misleading red cycles.

**5. Raise the reviewer inline cap 16→40 KB (amend fabro-9467's ceiling; durable fix is open fabro-e4c4).**
Evidence: the evidence capture was 34,108 bytes (evidence@1 script timing) — above the reviewer node's `preamble_inline_max_kb=16` — so it blob-ref'd and the reviewer burned a read_file round-trip, despite the graph's 48 KB budget existing precisely to kill blob detours (fabro-1e9f comment in `workflow.fabro`) and reviewer context peaking at 2.7% of the 1M window. Change: one attribute in `.fabro/workflows/develop/workflow.fabro` (reviewer node). Expected effect: per-seed captures render inline; one fewer tool round-trip per review; fabro-e4c4's output_schema delivery remains the endgame.

**6. Extend closeout re-filing to disclosed deferred actions — seed fabro-534e (open).**
Evidence: the run closed fabro-3916 with an outstanding human follow-up ("a Java-capable host should re-run `bun run generate` once to confirm parity") living only in `implementation_summary` and the journal; closeout (0.4s, `sd close` only) filed nothing. Change: extend the 534e mechanism in `.fabro/workflows/develop/scripts/closeout.nu` (re-file EXEMPTION bullets) to also file disclosed deferred/regen-confirm actions from the summary. Expected effect: deferred human actions stay tracker-visible instead of dying with the closed seed — exactly the failure mode 534e was filed for.

**7. Enforce the brief-quality gate on `sd show` re-fetch — seed fabro-4881 (open).**
Evidence: implementer@1 (events seq 94-96) re-fetched the full 3.7KB seed although the brief already carried 8 complete bulleted criteria; the prompt permits re-fetch only for thin/ambiguous briefs. Low impact (~1 call/pass) but it is the exact behavior 4881 targets.

**8. Re-check the pipeline-progress header — seed fabro-9e8b (open).**
Evidence from stage prompts: implementer@1 saw "Pipeline progress: 0 of 7" with planner already completed; implementer@2 saw "2 of 7" with five nodes completed. Either counting scheme is inconsistent — fresh counter-evidence for 9e8b, cosmetic but user-visible in every preamble.

Not inspected: the merged PR #205 body and any post-merge CI — outside this run's events.
