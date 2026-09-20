# Improve review — run 01M2W0MTZ032QBAJ2P49AK66FD

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (7.5 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-19 05:13+0000 by revisor `fabro_ask`

---

Recommendations for the develop workflow, ordered by expected impact. All facts below are from this run's events, checkpoints, stage journals, and transcripts (run `01M2W0MTZ032QBAJ2P49AK66FD`, seed fabro-c643, PR #264; total 7m02s active, $0.512, implementer = 72% of wall / 66% of cost).

---

**1. Batch the implementer's shell calls — it burned 300 s of inference on 4 s of tools.**
- What happened: implementer@1 ran 21 shell calls totaling 4,086 ms tool time against 299,747 ms inference (from run conclusion/stage timing). Single-purpose calls like `nu .fabro/scripts/dup-run-check.nu fabro-c643 --self …` (615 ms, seq 92) and `cat .fabro/Dockerfile.toolchain` (40 ms, seq 95) each cost a full ~25–30 s LLM round-trip.
- Change: implement **fabro-866a** ("Implementer: fold recon and verification rounds into chained shell calls") in `.fabro/workflows/develop/prompts/implementer.md` steps 1–2.
- Effect: ~5 fewer LLM turns ≈ 100–150 s off implementer wall per run (~15–20% of total run wall) — the single largest measured lever in this run.

**2. Fix the reviewer evidence truncation — the capture contract was violated in-run.**
- What happened: the evidence capture was 14,206 bytes (~230 lines); the reviewer's preamble rendered "(190 lines omitted)" and it had to reconstruct the full diff itself via `git diff 0fa14680` (3 shell calls; reviewer journal, checkpoint seq 272). This despite `preamble_inline_max_kb=16`, `preamble_output_max_lines=200`, and the 48 KB graph budget that fabro-1e9f raised specifically so per-seed captures render whole.
- Change: on the `reviewer` node in `.fabro/workflows/develop/workflow.fabro`, raise the evidence stage-section line cap (`preamble_output_max_lines` 200 → ≥400) or route over-budget captures through an explicit blob-ref marker instead of silent omission.
- New-seed justification: fabro-cf3e raises only the *KB* knob (`preamble_inline_max_kb` 16→32); the binding constraint observed here is the *line* cap, which no existing seed covers.

**3. Tell agents which binaries the sandbox actually has — the docker-build criterion was unrunnable.**
- What happened: the brief prescribed "docker build of the toolchain image succeeds (timeout_ms >= 600000)"; the run sandbox has no docker binary. The implementer probed, reported the only FAIL ("deferred, environment-gated") and journaled the painpoint; the reviewer re-adjudicated the same finding (implementer + reviewer journals, seq 218/272).
- Change: implement **fabro-0586** — add an available-binaries line to the develop PROJECT_FACTS ("docker: absent; image build/push belongs to the host `just run-images` / `just image-release` session"); **fabro-4be6** (planner dry-runs brief verification commands) is the enforcement arm.
- Effect: image seeds (fabro-cfd6 class is next) stop carrying unrunnable criteria — eliminates the probe calls, the FAIL arm, and the reviewer adjudication tax.

**4. Make closeout re-file deferred actions — this seed's actual payoff is currently tracked nowhere.**
- What happened: fabro-c643's value (host-side image build+push to `ghcr.io/denkhaus/fabro-toolchain` and the wall-time before/after smoke evidence) was deferred to "the host integrate session" and now lives only in the implementation summary and journal of a *closed* seed (closeout diff, seq 280). Nothing open reminds anyone to build/push the image or measure the 4–7 min saving.
- Change: implement **fabro-7aac** — `closeout.nu` sweeps deferred actions disclosed in `implementation_summary` into open seeds before `sd close`.
- Effect: the warm-image rollout gets a tracked owner; the loop's biggest wall-time lever doesn't silently stall in a closed seed's journal.

**5. Fix the preflight's truncated-anchor false positive.**
- What happened: the preflight flagged `.fabro/Dockerfile.toolchai` as `missing_file` (checkpoint seq 23) although the seed body spells the path correctly — the *script's* anchor extraction truncated it. The planner burned a shell call adjudicating it (seq 51–53) and the reviewer re-flagged it as a second painpoint. The same table also mis-flagged `operations/create.rs` (fabro-7611's crate-relative arm).
- Change: in `planner-preflight.nu` / `anchor_check.nu`, verify/normalize extracted paths after shell-quoting (the planner's own fix idea, journal seq 70).
- New-seed justification: fabro-7611 covers only crate-relative resolution; the truncated-extraction mechanism has no seed.

**6. Stop `ml record` from mutating loop config mid-run.**
- What happened: the lesson capture (mx-788723) added a `devloop` domain to `.mulch/mulch.config.yaml` plus a new expertise file — two loop-asset files of churn in the seed diff the reviewer had to classify as churn instead of seed work (implementer diff, seq 218).
- Change: implement **fabro-b94d** (pre-declare the `devloop` domain / move the config mutation out of run-time).
- Effect: every lesson-capturing implementer pass stops shipping bookkeeping churn; the evidence anomaly section shrinks by two files.

**7. Trim the planner's `sd ready` firehose.**
- What happened: the planner's first call returned all 200 ready seeds / 28.8 KB (`stdout_truncated: true`, seq 37–39) even though the preflight verdict table had already ranked the top 5; planner input was 50,339 tokens / $0.113 (22% of run cost).
- Change: implement **fabro-c3b4** (top-N `sd ready` view, e.g. `--limit 20`) in `.fabro/workflows/develop/prompts/planner.md` + the PROJECT_FACTS command table.
- Effect: smaller planner context, a few seconds and cents per run, and no truncation ambiguity in candidate selection.

**8. Surface stage cost/time and journal painpoints in the PR body.**
- What happened: PR #264's only public trace of unfinished work is the implementer's FAIL line; both journal painpoints (evidence truncation, docker absence) and the per-stage cost split live only in engine events.
- Change: implement **fabro-1409** — PR postlude appends a per-stage cost/time table and a journal-painpoint digest to the PR body.
- Effect: users and reviewers see friction and deferred items at merge time without opening the run — in this run, the docker FAIL and the pending image build would have been visible on the PR.

---

Not inspected: I could not read the raw blob contents of the tester's full gate log or the preflight script source in this session (blob refs and `.fabro/**` script internals were only visible via event payloads), so recommendations 2 and 5 rest on the journal/event evidence quoted above rather than on reading `evidence.nu`/`planner-preflight.nu` line by line.
