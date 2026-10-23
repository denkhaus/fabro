# Improve review — run 01M5JRAP1F67RZE674TBRPGPY8

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (23:30–23:50 Berlin on 2026-10-22, 19m56s, $2.05, revisor pass — reason and cost in run detail)
- generated: 2026-10-23 02:38+0200 by revisor `fabro_ask`

---

All evidence gathered. Tracker searches I ran to dedupe: `blob`, `anchor`, `title`, `spec-paths`, `paren`, and direct id lookups `fabro-85f6`, `fabro-9dd6`, `fabro-3ec9`, `fabro-1923`, `fabro-a827`, `fabro-17fb`, `fabro-801e`, `fabro-b769`, `fabro-c3b4`.

**Run baseline (from run events + conclusion):** 19m56s wall, $2.05 total. Implementer: 924.7s wall (77% of run), $1.80 (88% of cost), 891s inference vs 32s tools, 84 messages, 83 tool calls (61 shell/4 errors, 19 edit_file/2 errors). Planner 96s/$0.11, reviewer 83s/$0.14, tester 54s. Zero retries, zero gate bounces — the graph itself behaved; the waste is in what the deterministic tiers *failed to enforce* and in context volume.

---

**1. Resolve workflow-relative shorthand paths in claim-check/preflight — both coverage tiers were idle this run.**
Evidence: preflight flagged `revisor/workflow.fabro`, `develop/workflow.fabro`, `loop/workflow.fabro` as `missing_file` on 3 of 5 candidates (false positives — planner had to adjudicate each by hand); claim_check reported `spec-paths []` and "no range to scan — arm skipped (fail-open)"; the tester's seed-spec-paths tier then reported "tier idle". Net: zero deterministic one-unit enforcement, so implementer AND reviewer each hand-verified spec-path coverage against `git diff` (journaled by both).
Change: in `.fabro/workflows/develop/scripts/claim-check.nu` and `planner-preflight.nu`, resolve `<lane>/<file>` citations against `.fabro/workflows/<lane>/` before flagging/harvesting, so anchors stop false-flagging and spec-paths actually populate.
Effect: restores the lane's core deterministic guarantee; deletes the hand-verification pass from two LLM stages every run.
Seed: **new** — no open seed covers workflow-root resolution; `fabro-7611` (closed) fixed only the crate-relative analogue and `fabro-e8ae` is a one-char truncation fix.

**2. Make the evidence blob readable by the reviewer — the capture was dead weight this run.**
Evidence: evidence@1 emitted only `blob://sha256/7d978f0c…` (100,765 bytes); reviewer's journal painpoint: "no fabro_blob tool — the allow-list admits read_file/grep/glob only, so the capture was unreadable and the review fell back to verifying against the working tree." The approval basis was therefore the tree, not the capture.
Change: in `.fabro/workflows/loop/workflow.fabro`, reviewer node — grant the blob-read tool via `x.fabro_tools` (or make `evidence.nu --lane loop` inline a capped plain-text capture).
Effect: reviewer judges the captured evidence; the 2m-timeout evidence stage stops being pure overhead and approval can't drift from what was actually captured.
Seed: **new** — closeout filed only the comment-rot (`fabro-85f6`) and ask-wording (`fabro-9dd6`) findings; open blob seeds (`fabro-eed2`, `fabro-7613`) cover prompt-doc/engine-stamping, not the missing read grant on this node.

**3. Diff-scope the loop gate's prompt-lint output — 8.5KB of mostly-irrelevant warnings.**
Evidence: tester output was 8,514 bytes dominated by 35 prompt-lint warnings (conductor/develop date pins, routing-named schema warnings for untouched workflows) — this run touched 12 files, none of them conductor.
Change: `.fabro/workflows/loop/scripts/loop-gate.nu` — filter prompt-lint warnings to paths in the run diff.
Effect: gate output the implementer reads on bounce and the reviewer ingests in preamble shrinks ~10x; this exact pain is already scoped.
Seed: **fabro-a827** (open, @loop).

**4. Deduplicate the reviewer's tester-vs-evidence reading.**
Evidence: reviewer input was 46,903 tokens (35,155 conversation) for a review that verified 7 points — it read the tester section AND the evidence capture AND the implementation_summary covering the same ground.
Change: `.fabro/workflows/loop/prompts/reviewer.md` + preamble allow-keys — drop one of the two overlapping sections (keep evidence's recorded checks, drop the raw tester battery output once #3 lands).
Effect: ~10KB/review less context → cheaper, faster, less dilution of the actual review axes.
Seed: **fabro-17fb** (open, @loop).

**5. Ship the nushell-scripts skill for the implementer — nu footguns ate real inference.**
Evidence: implementer hit the documented `$\"…\"` literal-paren trap (runtime exit 1 vs intended exit 2, caught only by the fixture negative case — journaled painpoint), plus 6 errored tool calls across 61 shell/19 edit calls; 84 messages × ~10.6s inference each is the whole cost driver.
Change: add the skill per the existing seed; reference it in `.fabro/workflows/loop/prompts/implementer.md`.
Effect: fewer wasted round-trips and less re-derived nu semantics per edit — direct cut into the $1.80/15.4-minute implementer bill.
Seed: **fabro-d19e** (open, @loop, seen in this run's `seeds ready` output).

**6. Fix the run-title cap friction — 4 tool calls, 3 rejected.**
Evidence: planner called `fabro_run_set_title` 4 times with 3 errors (titles >100 chars rejected; prompt says "a few sentences"); the planner journaled "had to trim twice; suggest the planner prompt note the 100-char cap".
Change: land the cap fix per the seed, and meanwhile add the one-line 100-char note to the RUN TITLE paragraph in `.fabro/workflows/loop/prompts/planner.md`.
Effect: one call instead of four on every run that claims a seed.
Seed: **fabro-b529** (open — same rejection pattern, filed from the revisor lane; this run proves it's cross-lane).

**7. Sweep the revisor date pins this lane keeps re-flagging.**
Evidence: this run's own gate emitted 4 date-pin warnings on revisor files (pins from 2026-08-27/09-02/09-03/09-05, all >45 days) — and this run *edited* revisor files without sweeping them.
Change: re-validate or retire the stale pins in `.fabro/workflows/revisor/workflow.fabro` / `workflow.toml` per the seed.
Effect: removes 4 of the 35 gate warnings permanently; while at it, fix the env_guard comment rot (lines ~29-31, "shell out to nu and seeds" — seeds is gone) already filed as **fabro-85f6**.
Seed: **fabro-801e** (open, @loop).

**8. Long-stage heartbeat — the run was a 15-minute black box.**
Evidence: implementer ran 21:32:20→21:47:46 with nothing user-visible between checkpoints; total run 19m56s. Slack notifications fire only on terminal events.
Change: per the seed, emit a heartbeat notification for stages exceeding a threshold.
Effect: an operator watching #dev-fabro sees liveness instead of inferring it — the single biggest UX gap in an otherwise clean run.
Seed: **fabro-b769** (open, @loop).

**9. Top-N the planner's ready view.**
Evidence: `seeds ready --limit 200` dumped 64 issues / 9.2KB into the planner's context; the planner only needed the top candidate by priority.
Change: per the seed, use a top-N view in `.fabro/workflows/loop/prompts/planner.md` PROJECT_FACTS.
Effect: smaller planner preamble (23.6k input tokens this run) and less adjudication surface.
Seed: **fabro-c3b4** (open, @loop).

---

Not inspectable with my tools: the blob's actual content (`blob://sha256/7d978f0c…` lives in the blob store, which my toolset can't read) and the PR-body generation transcript beyond its success record (PR #466, glm-4.7, 2m10s). Nothing in either gap changes the recommendations above.
