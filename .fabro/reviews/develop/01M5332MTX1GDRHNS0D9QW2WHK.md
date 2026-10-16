# Improve review — run 01M5332MTX1GDRHNS0D9QW2WHK

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (437 s wall ≈ 7.3 min, revisor pass — reason and cost in run detail)
- generated: 2026-10-16 19:49+0000 by revisor `fabro_ask`

---

All evidence is in hand. Here are the recommendations, ordered by expected impact, each grounded in what this run actually did (seed `fabro-020b`, 12 stages, 437 s wall, $0.5717 total — 79% of it in the implementer; all timings/costs from the run's conclusion record).

---

**1. Drop the tester gate transcript from the reviewer preamble — graph change, highest fidelity win.**
- **What happened:** The reviewer's own journal (from run events, reviewer@1) says its "prompt preamble was truncated at the 48 KB ceiling, cutting off the tail of the develop reviewer.md input text… later input sections were judged from the loop-lane copy." The tester output (12,292 B) + evidence capture (38,556 B) alone are ~50.8 KB — over the `x.preamble_budget_kb=48` in the graph before any prompt text. Because `DOC_EXTENSIONS` sorts `.md` last in the diff walk, the cut tail was exactly the two `reviewer.md` contract diffs this seed changed — the reviewer adjudicated its own contract change from a truncated copy.
- **Change:** In `.fabro/workflows/loop/workflow.fabro`, reviewer node: `x.preamble_stages_ignore="planner,implementer,reviewer"` → add `tester`. The loop `prompts/reviewer.md` already says "The loop gate was green… Do NOT re-derive those checks", so the 12.3 KB transcript is dead weight; the attribute semantics (omits those stages' history sections from that node's preamble) are proven in closed seed `fabro-a85b`/`fabro-e47c` (from `.seeds/issues.jsonl`).
- **Expected effect:** Frees ~12.3 KB — the preamble fits budget, and the reviewer sees the complete capture including the docs-last diffs that were being cut. Removes a silent review-fidelity risk on every mixed capture.
- **Seed:** New seed needed — the budget lineage (`fabro-8d2c` → superseded by `fabro-1e9f`, which set the current 48) is closed, and no open seed covers loop-lane adoption of tester-stage scoping for the reviewer; raising the budget again would just re-breach on the next big capture.

**2. Stop running the lints twice per gate and quiet repeated prompt-lint notes.**
- **What happened:** In tester@1's output, `lint-nu` and `prompt-lint` each ran **twice** — once inside `just validate-workflows` (tier 1) and again inside `battery-runner` (tier 2) — and the ~4.5 KB block of synthetic-fixture-id notes + date-pin/routing-named warnings printed twice in the tester output *and a third time* inside the evidence capture's recorded checks. That's ~13 KB of pure repeated noise riding the reviewer preamble path, plus duplicated work inside the 19.6 s gate wall. The graph comment's claim "battery output is small enough to inline" is no longer true (12.3 KB).
- **Change:** In `.fabro/workflows/loop/scripts/loop-gate.nu` (tiers at lines 83–84): make tier 1 graph-only (add a `--no-lint` flag to `scripts/validate-workflows.nu`, or have loop-gate call battery-runner as the sole lint surface), and give `.fabro/scripts/prompt-lint.nu` a summary mode (counts once, notes/warnings listed once per run, not per invocation).
- **Expected effect:** Gate output drops to ~4 KB (genuinely inlineable for the implementer's red-bounce path), gate wall shrinks measurably, and ~13 KB of noise leaves the reviewer preamble — compounding fix #1.
- **Seed:** New seed needed — `fabro-eae5` (battery-runner unification) is landed, and no open seed covers double-running the lint tiers or deduplicating advisory notes across a single gate pass.

**3. Add a byte-level guard to `cap-changed-lines` — line count ≠ bytes.**
- **What happened:** The implementer's journal observation (from run events): "the 30-CHANGED-LINE cap bounds line COUNT, not bytes, so a pathological single multi-KB line still passes through — HARD_CAP remains the only byte-level guard," with the seed's own basis capture at 94,873 bytes dominated by `.seeds/issues.jsonl` hunks that embed whole seed bodies on single lines. `fabro-020b`'s stated goal ("typical capture lands under the reviewer's 16 KB inline ceiling") is therefore not yet guaranteed for the exact pathology that motivated it.
- **Change:** In `.fabro/workflows/develop/scripts/evidence.nu`, `cap-changed-lines`/`capped-diff-walk`: truncate any single changed line past N bytes (e.g., 512) with a `…line truncated` marker, counted in the disclosure; extend the `cap-changed-lines` battery in `evidence-smoke.nu` accordingly.
- **Expected effect:** Churn/anomaly captures stay under the 16 KB inline ceiling even when seed bodies ride single lines — closing the residual of the seed this run just shipped.
- **Seed:** New seed needed — `fabro-020b` closed today with this exact residual observation journaled but unfiled (closeout filed only `fabro-41fa`), and `HARD_CAP` is aggregate-only; no open seed covers per-line/per-file byte capping.

**4. Turn the nu `$\"...\"` literal-paren trap into a lint hint / conventions line.**
- **What happened:** The implementer's first verification pass failed twice (recorded checks in the evidence capture): `just lint-nu` parse-FAILED with misleading "Module not found" spans in `evidence.nu` + `evidence-smoke.nu` (actually the literal `(truncated: …` inside an interpolated string), and the smoke died once on `Command 'or' not found` (unparenthesized `or`). Tool stats: 2 shell + 2 edit_file errors. The journal painpoint asks for exactly this fix: "a one-line note in the develop workflow's shared nu conventions (or a lint-nu hint)… would save the next run the same diagnosis."
- **Change:** In `scripts/lint-nu.nu`, append a hint to the parse-failure diagnostic ("literal `(` inside `$\"…\"` strings is an expression opener — avoid or escape"), and/or add the one-line rule to the nu-conventions section of `.fabro/workflows/develop/prompts/implementer.md` (+ the loop copy, per the one-unit rule the run itself just exercised).
- **Expected effect:** The next implementer fixes paren-in-`$\"...\"` in one pass instead of burning a diagnosis detour — this run that cost two failed tool calls inside the stage that is 79% of run cost ($0.45 of $0.57).
- **Seed:** New seed needed — `fabro-88f2` (closed) added *detection* of interpolated-paren bugs to lint-nu but no hint/convention; open `fabro-2904` covers bash double-quote interpolation of `nu -c` snippets, a different mechanism. The painpoint from this run is unfiled.

**5. Work `fabro-41fa` next: surface non-zero `git diff` exits as errors, not "UNSEEN".**
- **What happened:** The reviewer's non-blocking nit (journal, reviewer@1): `capped-diff-walk`/`diff-walk` drop a file from `included` when `git diff` exits non-zero, "so a non-zero diff exit on a listed file surfaces as 'UNSEEN' rather than an error" — a failing diff silently shrinks review scope. Closeout filed it this run as **`fabro-41fa`** (open, assignee `loop`, confirmed in `.seeds/issues.jsonl`).
- **Change:** In `.fabro/workflows/develop/scripts/evidence.nu`, both walk functions' `if $res.exit_code != 0 { continue }` arms: emit an explicit `diff-error` line naming the file and exit code instead of counting it as omitted.
- **Expected effect:** A genuinely broken diff command can no longer masquerade as a cap omission the reviewer must hand-adjudicate — protects the integrity of the capture that recommendation #1 just made the reviewer's complete input.
- **Seed:** existing — `fabro-41fa`.

---

**Not recommended (checked, no basis in this run):** the graph's exit-kind topology worked exactly as designed (no deadlocks, no retries, `total_retries: 0`); planner cost ($0.08, 8 shell calls) and evidence latency (386 ms) are already lean; caching is healthy (1.13 M cache-read tokens in the implementer). The cheapest remaining wins are the five above — #1 and #2 together remove ~25 KB from a 48 KB budget that was breached by ~3 KB of prompt text alone.
