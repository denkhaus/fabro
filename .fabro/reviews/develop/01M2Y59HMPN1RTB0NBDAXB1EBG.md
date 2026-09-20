# Improve review — run 01M2Y59HMPN1RTB0NBDAXB1EBG

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (6.8 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-20 01:12+0000 by revisor `fabro_ask`

---

All findings below are from this run's events (stage timings, usage, tester failure output, planner/implementer journals) and the tracker listing captured in the planner's `sd ready` call. Run totals for context: 6 m 40 s wall, **$0.497** LLM cost, one seed (fabro-2ab8) claimed → implemented → gate-red bounce → re-implemented → approved → closed, PR #302.

---

**1. Catch the two defect classes that caused the only gate red *inside the implementer lane*, not at the gate.**
What happened (from run events): implementer@1 appended new smoke assertions to `closeout-smoke.nu` **after** the trailing `exit 0` (dead code — the smoke still printed "ok", a false green) and used a fabricated literal `fabro-9abc`, which prompt-lint rejects. tester@1 went red at 15.5 s; the repair lap (implementer@2: 148.7 s, $0.183 = 37 % of run cost, plus a second 23 s gate run) made the bounce the single most expensive event of the run (~40 % of cost, ~3 of 6.7 min). `lint-nu` was green because dead code still parses.
Change: (a) add a rule to `.fabro/scripts/prompt-lint.nu` flagging any statement after a top-level `exit 0` in sourced smoke scripts; (b) make `just verify implementer` (`scripts/verify.nu`) run prompt-lint over changed loop-asset files so unresolvable seed-id literals die pre-gate.
Expected effect: this exact bounce class costs $0 instead of ~$0.19 + 3 min per occurrence.
**New-seed justification:** no existing seed covers deterministic detection of dead-code-after-`exit 0` or pre-gate seed-literal resolution (closest, fabro-50f8/fabro-f18a, lint other classes); the implementer's own journal painpoint in this run proposes exactly this fix.

**2. Make the in-flight exclusion run-status-aware — a terminal-failed run's branch claim must not block re-claiming.**
What happened (from preflight output + planner reasoning trace): the top candidate **fabro-af22 (High bug: expand_skill crashes stage prompts)** was marked `in_flight` via the unmerged branch of run 01M2Y1W8Y…, which had **failed** 53 minutes earlier with no PR. The planner explicitly wrestled with the contradiction ("Run is failed (terminal), no PR → not in-flight per fabro_runs_list… to be safe, skip") and fell through to a Medium cosmetic residual seed. It journaled: "a later pass may need to adjudicate whether failed-run claims should block re-claiming."
Change: in `.fabro/workflows/develop/scripts/planner-preflight.nu`, resolve the owning run's status (via the `inspects` catalog) and only mark `in_flight` for non-terminal runs or open PRs; emit `abandoned: true` for terminal-failed owners so the planner may re-claim.
Expected effect: High-priority seeds stop starving behind dead runs; this run would have taken fabro-af22 instead of fabro-2ab8.
**New-seed justification:** closest seeds (fabro-4bb7 PR-state backfill, fabro-01af claim race) fix the projection/race, not the preflight's run-status blindness.

**3. Put a materiality bar on the closeout residual sweep — it just spawned a full dev cycle over a trailing blank line.**
What happened (from closeout diff, run events): this run closed fabro-2ab8 and the sweep re-filed **fabro-168b** from the reviewer's "Cosmetic nit not blocking: a stray trailing blank line." Each such seed buys a complete run (~$0.50, ~7 min, one PR). This run itself existed only because fabro-22fa's sweep filed fabro-2ab8 the same way — the mechanism is self-perpetuating.
Change: in `closeout.nu` (`sweep-reviewer-findings` / `is-nonblocking`), file only findings that name a defect with a target; cosmetic-only nits stay journaled (or file at P4 with a `cosmetic` label so they can't outrank user work).
Expected effect: kills a recurring ~$0.50/7-min work generator per non-defect observation; fabro-168b would not exist.
**New-seed justification:** existing seeds cover the sweep's trigger sources and channels (fabro-7aac, fabro-534e, fabro-89dd), none sets a materiality threshold.

**4. Bound the planner's two context firehoses — planner was 43 % of run cost.**
What happened (from agent events): planner@1 cost $0.214 of $0.497 (98.8 s). Its first `sd ready` returned the full 200-seed listing (28,420 bytes, **truncated** in the tool result), and `fabro_runs_list` was called **without `created_since`**, returning 101 full run JSON objects; conversation tokens jumped 7.5 k → 51.6 k across those two calls. It also made one dead probe to nonexistent `.fabro/prompts/`.
Change: adopt **fabro-6b58** (created_since window + self-exclusion on the in-flight `fabro_runs_list` call) and **fabro-c3b4** (top-N `sd ready` view; the preflight already needs only the top 5), plus **fabro-55a7** (batch recon greps into one shell call).
Expected effect: planner input/cache tokens (63 k input + 430 k cache-read here) drop meaningfully; at this run's mix, tens of percent off the largest single stage cost.

**5. Critical-first gate output and silence intentional routing-schema warnings.**
What happened (from tester@1 failure output): the one real error ("seed id 'fabro-9abc' does not resolve") was printed **after 11 standing warnings**, 10 of which warn that `planner-output.schema.json` / conductor schema use routing-named top-level properties — intentional by design (fabro-9ec3 arm 3) and re-fired identically on both gate runs.
Change: implement **fabro-e988** (critical-first failure summary in `scripts/qualitygate.nu`), and add an intent marker/allowlist in `.fabro/scripts/prompt-lint.nu` for deliberately-routing schemas.
Expected effect: red-gate diagnosis (implementer@2 had to page past the noise) becomes one-glance; recurring false-positive warnings stop eroding trust in the gate log.

**6. Gate-bounce matched 2 irrelevant seeds out of 3 hits.**
What happened (from gatebounce output): the bounce attached fabro-41de (relevant: seed-id literal lint) plus fabro-f18a and fabro-9495 (irrelevant), inflating the implementer@2 preamble.
Change: implement **fabro-c841** (match on error signatures, not generic terms) in `.fabro/workflows/develop/scripts/gate-bounce.nu`.
Expected effect: bounce prompts carry only root-cause candidates; less misdirection during the most expensive pass.

**7. Ship the nushell-scripts skill — this run paid the tuition again.**
What happened (from implementer@1 journal): a smoke rerun was burned on `"x" * 80` being a parse-time type error in nu 0.115 — the exact re-derivation class already recorded as mx-66dfc1 in a prior run.
Change: implement **fabro-d19e** (vendored nushell skill loaded by the implementer like the rust-style-guide).
Expected effect: eliminates the recurring one-rerun nu-semantics tax on loop-asset seeds.

**8. Require defect+target in auto-filed residual bodies.**
What happened (from planner tool calls): fabro-2ab8's body was finding-text-only; the planner spent ~6 shell calls (sd show fabro-22fa, journal grep, two closeout.nu greps, `sd create --help`, smoke read) re-deriving what the defect even was — every residual seed arrives this way by construction, since the sweep files the reviewer's observation verbatim.
Change: in `closeout.nu`'s filing (or the reviewer journal contract in `.fabro/workflows/develop/prompts/reviewer.md`), emit/require a `Defect:` + `Target:` pair in the filed description.
Expected effect: removes the ~1-minute planner re-derivation lap per residual seed; fabro-2ab8's correction step becomes unnecessary.
**New-seed justification:** fabro-7f27 requires bodies on revisor-filed seeds; no seed requires an actionable defect+target in closeout-filed residual bodies.

**What I could not inspect:** the merge-target branch state of failed run 01M2Y1W8Y (af22/b03f's actual claim status there) and PR #302's post-run state — those live outside this run's events.
