# Improve review — run 01M40KWWZ4M4P396PGCVH08Q6G

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (16m51s, revisor pass — reason and cost in run detail)
- generated: 2026-10-04 18:18+0000 by revisor `fabro_ask`

---

All evidence is in. Here are the recommendations, ordered by expected impact, each grounded in this run (01M40KWWZ4M4P396PGCVH08Q6G: wall 16m51s, cost $1.56, all 11 stages first-pass green, 0 retries; implementer = 827s / $1.399 = 82% of wall / 90% of cost; reviewer = 124s / $0.112; planner = 48s / $0.048 — from run events/conclusion).

---

**1. Make the seed-work diff actually reach the reviewer — re-size the reviewer's evidence caps (graph design / UX).**
What happened: the evidence capture was 58,469 bytes / ~1,600 lines, but the reviewer node caps inline preamble at 16 KB / 200 lines (`workflow.fabro:408,414`), so the reviewer opened with "inline diff was truncated (1488 lines omitted)" and re-verified everything against the working tree via 12 `read_file` calls (from run events, reviewer@1 tools: 12 read_file / 4 glob / 2 grep). It journaled this as a painpoint: the review then "depends on tree state matching the diff-base." The caps were sized when captures were "~120 lines / ~14 KB" (fabro-meta-c9f2, fabro-9467 comments in the graph); this seed's capture is 4× that, and reviewer context sat at 3.4% of the 1M window — the caps, not the window, are the constraint (the graph's own fabro-1e9f logic).
Change: in `.fabro/workflows/develop/workflow.fabro` (reviewer node) raise `x.preamble_output_max_lines` 200 → ~1,700 and `x.preamble_inline_max_kb` 16 → 64; optionally also demote generated files (Cargo.lock, +170 lines here) to counts-only in `.fabro/workflows/develop/scripts/evidence.nu` `diff-walk`, the way loop-churn already is.
Expected effect: reviewer verifies from the capture directly — fewer tool round-trips (this run's reviewer was 12% of wall for a checklist), and the tree-vs-diff-base correctness hazard the reviewer itself flagged is gone.
Seed: **new seed needed** — the tracker's only seeds are the four ADR-0023 product seeds (mulch-70ed/0e39/96ed/b0f7); none covers develop-loop assets under `.fabro/`.

**2. Stop the guaranteed per-run preflight false positive — classify tracker-only diffs as filed-only in dup-run-check (error handling).**
What happened: preflight@1 returned `verdict: "duplicate"` for mulch-70ed because commit 7c396e6 ("seeds: assign mulch-70ed @fabro (line dispatch, first CLI-fired pass) (#6)") names the seed id and carries a squash `(#6)` suffix — but its diff is confined to `.seeds/issues.jsonl`. Both the planner ("a false positive in spirit… adjudicated as not-landed") and the implementer (painpoint: "the two verdicts disagree mechanically… git show --stat verified") burned cycles adjudicating it — double adjudication across two LLM stages. This is systematic: every seed this line dispatches gets an "assign … (#n)" commit, so every future run re-hits it. `dup-run-check.nu:91` classifies filed-only by *subject* regex (revisor/file/reopen/verify/move/close) — "assign" isn't in it.
Change: in `.fabro/scripts/dup-run-check.nu`, extend the fabro-395b tracker-bookkeeping classification from subject-regex to diff-scope: a candidate match whose `git show --stat` diff is confined to tracker paths (`.seeds/**`, `.mulch/**`) classifies filed-only, never "duplicate".
Expected effect: verdict table reads "clean" for freshly-assigned seeds; the planner's already-landed lap shortens every run and the implementer never re-verifies; also removes the residual risk of a future planner routing "Already landed" off an assignment commit.
Seed: **new seed needed** — same justification: no existing mulch-* seed covers `.fabro/scripts/` loop tooling.

**3. Sharpen the stale-binary rule's trigger in the implementer prompt (prompting / tool efficiency).**
What happened: the implementer journaled "Stale-binary trap hit twice this pass (rule c): nextest re-ran a stale test binary after edits (0.01s 'Finished', old failure text re-emitted); touching the changed file forced the rebuild." Rule (c) already exists (`prompts/implementer.md:31`) but only triggers *after* "a test failure that survives an obvious fix" — the model diagnosed twice before applying it, on the stage that is 82% of wall and 90% of run cost. The reliable signal was available earlier: a nextest invocation that "finishes" in ~0.01s with no compile step.
Change: in `.fabro/workflows/develop/prompts/implementer.md` rule (c), add the mechanical pre-trigger: "a nextest run that returns in ~0.01s reporting 'Finished' without compiling = stale binary; touch the changed file / `cargo clean -p <crate>` and re-run *before* diagnosing."
Expected effect: eliminates the 2 wasted diagnose-rerun cycles observed here on the costliest stage; compounding across every Rust seed (0e39, b0f7 next).
Seed: **new seed needed** — prompt changes to the develop loop aren't covered by any product seed.

**4. Make gate-green prove the reference battery ran (error handling / gate integrity).**
What happened: the reviewer observed that `crates/mulch/tests/reference_roundtrip.rs` "silently skip[s] when no `ml` is on PATH, so a green gate does not prove the battery executed" — the `reference_ml()` else-branch just prints "skipping" to stderr and returns. That battery IS the ADR-0023 acceptance gate for mulch-70ed, so a warm-cache green gate (tester@1 ran in 3.3s) can mask a skipped acceptance proof.
Change: in `crates/mulch/tests/reference_roundtrip.rs`, replace the silent-return with a machine-greppable marker (e.g. print `MULCH-REF-BATTERY-RAN <ml-path>`), and have `scripts/qualitygate.nu` fail (or loudly warn) when the mulch crate's suite output lacks the marker.
Expected effect: the ADR-0023 acceptance gate can no longer silently degrade to "skipped, still green."
Seed: **mulch-0e39** (existing, now unblocked — CLI parity explicitly owns "a differential battery against a provisioned ml 0.10.7 reference"; the reviewer itself routed the suggestion to this seed).

**5. Collapse the 15 identical routing-named warnings out of every gate log (UX / noise).**
What happened: tester@1's entire 3.2 KB output was ~60% warning lines — 15 identical "top-level property 'X' is routing-named" warnings for the three *deliberately* routing schemas (planner-output, develop-output, survey-output; 5 properties each), emitted by `.fabro/scripts/prompt-lint.nu` check 5 (`routing-field-schema-warnings`, line 96) via `scripts/qualitygate.nu:131`. The planner output schema *must* carry those fields (fabro-9ec3 gave it teeth), so the warnings are permanent noise — and the tester section is re-read by the reviewer as a preamble section every cycle.
Change: in `.fabro/scripts/prompt-lint.nu`, allowlist routing-named top-level properties for `*-output.schema.json` files under `.fabro/workflows/` (or emit one summary line: "3 routing schemas, 15 intentional routing fields").
Expected effect: gate output (and the reviewer's tester section) carries signal only; warning-fatigue stops burying the real warnings this check exists for.
Seed: **new seed needed** — loop-asset lint hardening; no product seed covers it.

---

**What I could not verify:** per-tool-call timings inside implementer@1 aren't separable (aggregate tool_time 30s), so the exact cost of the stale-binary cycles in recommendation 3 is an estimate, not a measurement; PR-creation cost (glm-4.7, PR #7) isn't exposed in the run record. Everything else above is quoted from run events (stage outputs, journals, timings) and workspace files (`workflow.fabro`, `evidence.nu`, `dup-run-check.nu`, `implementer.md`, `prompt-lint.nu`, `.seeds/issues.jsonl`).
