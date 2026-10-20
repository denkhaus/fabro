# Improve review — run 01M5CVG6XT9ABZFTMGY45QGCKT

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (559s, revisor pass — reason and cost in run detail)
- generated: 2026-10-20 14:46+0000 by revisor `fabro_ask`

---

## What this run actually did (grounding)

From run events and the stage conclusion: the run claimed seed **fabro-7f58** (narrow `scripts/verify.nu`'s implementer test tier to name-filtered runs for purely-added test files), went **planner → claim_check → implementer → tester(RED) → implementer → tester(green) → evidence → reviewer(approved) → closeout**. Wall **9m19s**, inference 471s, cost **$0.527**. The two implementer passes alone were **$0.392 (74% of run cost)**. Tracker searches run for dedup: `seed-spec-paths`, `spec-named`, `nushell`, `verify exit`, `LAST_VERIFY_FAIL`, `run_search`, `test-fn-names`, `name filter`, `routing-named`, `prompt-lint warnings`, `heartbeat`, `title rejects`, `gate bounce`.

## Recommendations, by expected impact

**1. Move the seed-spec-paths check into the implementer's cheap lane — kill the tester bounce.**
What happened: tester@1 went RED on `seed-spec-paths [fabro-7f58] — 3 path(s) named, 1 missing` (`planner.md` not in diff), so the entire second implementer pass (98.5s, $0.115 = **22% of run cost**) plus a second full tester run (14.9s) existed to add one mirror line. The implementer@2 journal painpoint proposes exactly this fix.
Change: extend `claim_check` (`.fabro/workflows/develop/scripts/claim-check.nu`, which already parses the seed body's paths) to emit the spec-named path list into context, and add one line to `.fabro/workflows/loop/prompts/implementer.md`: before routing Implemented, diff that list against `git diff --name-only`.
Expected effect: this bounce class (~2 min + ~$0.12/run) disappears; the tester battery runs once.
Seed: **new seed** — the only spec-paths seeds (`fabro-8a60`, `fabro-48f3`) are closed and gate-side; none moves the check upstream into the implementer's own lane (the journal painpoint from this run is unfiled).

**2. Ship the nushell-semantics skill — stop paying the probe-iteration tax.**
What happened: the implementer@1 recorded checks show **five probe attempts, four failed** (`file_not_found` on path handling, `duplicate_command_def` from sourcing a `def main` file, `assert` not a command, `assert equal` parse mismatch) before the working probe — pure re-derivation of nu 0.115 semantics, inside the stage that consumed 74% of run cost (3 of 16 shell calls errored, per the stage tool stats).
Change: land the skill per open seed **fabro-d19e** and reference it from the probe section of the develop implementer prompt.
Expected effect: probe iterations drop from 5 to 1–2 on nu-script seeds; minutes of implementer inference saved per run.
Seed: **fabro-d19e** (open, @loop).

**3. Make `verify.nu` exit 1 on FAIL — close the silent-false-green hole.**
What happened: implementer@1's journal observation (verified in the file just now): FAIL branches print `verify: FAIL`, set `$env.LAST_VERIFY_FAIL`, and `return` — but nothing consumes that env var (lines 219–283 of `scripts/verify.nu`) and the script still exits 0, while the header claims "Exit 0 = all checks green". A caller trusting exit status reads red as green.
Change: in `scripts/verify.nu`, `exit 1` on every FAIL arm (fmt/clippy/compile/nextest incl. `full-crate-suite`'s `return false` path), drop the dead env var, and mirror the nonzero-exit contract in develop's `prompts/implementer.md` step 4 (same pattern as **fabro-7808**'s "grep exit 1 = expected no-match" rule).
Expected effect: implementer self-assessment and any future `just` wiring can trust exit status; removes a false-green failure class from the exact lane this run modified.
Seed: **new seed** — searches for `verify exit` / `LAST_VERIFY_FAIL` return zero matches; no existing seed touches verify.nu's exit contract.

**4. Fix the `test-fn-names` attribute window against blank lines.**
What happened: reviewer@1's journal found that the window resets on any non-`#[` line, so `#[test]` + blank line + `fn` silently drops that test from the name filter — the narrowed tier this run shipped can under-cover.
Change: in `scripts/verify.nu` `test-fn-names`, clear `$attrs` only on non-attribute **non-blank** lines.
Expected effect: no silently dropped test names in the new narrowed tier (currently bounded only because the deterministic tester re-runs suites).
Seed: **new seed** — `test-fn-names` returns 0 matches; `name filter` matches only the now-closed fabro-7f58 itself. The edge was discovered by this run's reviewer in code that landed minutes ago.

**5. Fix the `fabro_run_search` status contract at the planner's first call.**
What happened: the planner's in-flight check called `fabro_run_search` with status `["queued","running","awaiting_input"]` and got `unknown run status 'queued'` — 2 of its 3 run_search calls errored (per stage tool stats), each error costing an LLM round-trip in a 68s stage. (Same friction class: `fabro_run_set_title` rejected a >100-char title, 1 of 2 calls errored.)
Change: land open seed **fabro-1cde** (error names valid `RunStatusKind` values) and inline the valid enum literals into the step-5 in-flight bullet of `.fabro/workflows/loop/prompts/planner.md`.
Expected effect: the in-flight check succeeds on the first call every planner visit; one fewer wasted round-trip per run.
Seed: **fabro-1cde** (open).

**6. Quiet the known-benign battery warnings clogging gate output.**
What happened: each tester run prints ~9KB, dominated by 25 identical `routing-named` schema warnings, 19 date-pin warnings, and repeated synthetic-fixture-id notes — printed verbatim in tester@1, tester@2, the implementer's prompt-lint probe, and again inside the 37KB evidence capture the reviewer reads. They are static and benign every run.
Change: in `.fabro/scripts/prompt-lint.nu`, mark routing-named top-level props as intentional via a marker (the existing `SYNTHETIC_IDS_MARKER` pattern — cf. open seed **fabro-0d16** for its name drift) and collapse per-file fixture-id notes to one summary line.
Expected effect: gate output and evidence captures shrink roughly in half; the reviewer's 48KB preamble budget goes to signal, and RED lines like seed-spec-paths stop scrolling.
Seed: **new seed** — `routing-named` and `prompt-lint warnings` return 0 matches; fabro-0d16 covers only the marker-name drift, not quieting these warning classes.

**7. Heartbeat for long stages.**
What happened: implementer@1 ran **278s (~4.6 min)** with nothing user-visible — Slack notifications fire only on terminal events (`run.completed`/`run.failed`, per the run spec), so #dev-fabro was silent from 14:31:54 to 14:36:33.
Change: land open seed **fabro-b769** (long-stage heartbeat notification).
Expected effect: humans watching the channel see progress on multi-minute stages instead of assuming a stall.
Seed: **fabro-b769** (open, @loop).

**Not inspected / out of scope:** I could not measure cache-write behavior (0 `cache_write` tokens on every stage while `cache_read` was ~990K — possibly a provider-accounting artifact, not actionable from run data alone), and PR-body generation cost (glm-4.7, post-exit) isn't broken out in the conclusion.
